//! SFTP operations: upload, download, home directory detection, and connectivity probes.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use russh::client::Handle;
use russh_sftp::client::{RawSftpSession, SftpSession};
use russh_sftp::protocol::{Packet, StatusCode};
use tokio::io::AsyncWriteExt;

use super::session_pool::SshHandler;
use crate::config::schema::ShellType;

/// Resolve a remote path, expanding a leading `~` using the provided home directory.
pub fn resolve_remote_path(remote: &str, home_dir: &str) -> String {
    if remote == "~" {
        home_dir.to_string()
    } else if let Some(rest) = remote.strip_prefix("~/") {
        format!("{}/{}", home_dir.trim_end_matches('/'), rest)
    } else {
        remote.to_string()
    }
}

/// Retrieve the remote home directory by running `echo $HOME` (sh) or equivalent.
pub async fn remote_home_dir(
    handle: &Handle<SshHandler>,
    shell: ShellType,
    timeout: Duration,
) -> Result<String> {
    let cmd = match shell {
        ShellType::Sh => "echo $HOME",
        ShellType::PowerShell => "Write-Output $env:USERPROFILE",
        ShellType::Cmd => "echo %USERPROFILE%",
    };

    let out = super::session_pool::exec_on_handle(handle, cmd, timeout).await?;
    Ok(out.stdout.trim().to_string())
}

/// Open an SFTP session on the given SSH handle.
/// Unbounded: callers use `session_pool::open_sftp_bounded`.
pub(crate) async fn open_sftp(handle: &Handle<SshHandler>) -> Result<SftpSession> {
    let channel = handle
        .channel_open_session()
        .await
        .context("Failed to open SFTP channel")?;

    channel
        .request_subsystem(true, "sftp")
        .await
        .context("Failed to request SFTP subsystem")?;

    SftpSession::new(channel.into_stream())
        .await
        .context("Failed to create SFTP session")
}

/// Size of each read/write step in [`copy_idle`]; each step gets the full idle timeout.
const COPY_CHUNK: usize = 256 * 1024;

/// Run one transfer step under the idle `timeout`.
async fn step<T, F>(timeout: Duration, what: &str, fut: F) -> Result<T>
where
    F: std::future::Future<Output = std::io::Result<T>>,
{
    tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| anyhow::anyhow!("{what} timed out (no progress for {}s)", timeout.as_secs()))?
        .with_context(|| what.to_string())
}

/// Stream `r` into `w`, bounding each chunk (not the whole copy) by `idle`:
/// a slow transfer that keeps moving never times out, a stalled one does.
async fn copy_idle<R, W>(r: &mut R, w: &mut W, idle: Duration) -> Result<u64>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut buf = vec![0u8; COPY_CHUNK];
    let mut total = 0u64;
    loop {
        let n = step(idle, "read", r.read(&mut buf)).await?;
        if n == 0 {
            return Ok(total);
        }
        step(idle, "write", w.write_all(&buf[..n])).await?;
        total += n as u64;
    }
}

/// Suffix marker of upload temp files: `<dir>/.<name>.sshi-tmp.<pid>-<n>`.
const TEMP_MARKER: &str = ".sshi-tmp.";

/// Sibling temp name for `dest`: `<dir>/.<name>.sshi-tmp.<pid>-<n>`. The
/// per-upload counter keeps concurrent uploads of one path apart — e.g. to
/// several hosts sharing an NFS home, which would otherwise write the same
/// temp file and fail or corrupt each other (B77).
fn temp_sibling(dest: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (dir, name) = match dest.rfind(['/', '\\']) {
        Some(i) => dest.split_at(i + 1),
        None => ("", dest),
    };
    format!("{dir}.{name}{TEMP_MARKER}{}-{n}", std::process::id())
}

/// A temp file left by another sshi process that died mid-upload (SIGKILL):
/// our name pattern (`<pid>-<n>`, or the older bare `<pid>`), a pid other
/// than ours, and not modified for `max_age` seconds (so a concurrent run's
/// live temp file is never touched).
fn is_stale_temp(file_name: &str, mtime: Option<u32>, now: u64, max_age: u64) -> bool {
    let Some((base, tag)) = file_name.rsplit_once(TEMP_MARKER) else {
        return false;
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let (pid, counter_ok) = match tag.split_once('-') {
        Some((pid, n)) => (pid, digits(n)),
        None => (tag, true),
    };
    base.len() > 1
        && base.starts_with('.')
        && digits(pid)
        && counter_ok
        && pid != std::process::id().to_string()
        && mtime.is_some_and(|m| now.saturating_sub(u64::from(m)) >= max_age)
}

/// Age after which another process's temp file counts as abandoned.
pub(crate) const STALE_TEMP_SECS: u64 = 3600;

/// Remove abandoned upload temp files ([`is_stale_temp`]) in `dir`. Best
/// effort: listing or removal errors are logged, never fatal. Returns the
/// number removed.
pub(crate) async fn sweep_stale_temps(sftp: &SftpSession, dir: &str, timeout: Duration) -> usize {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let entries = match tokio::time::timeout(timeout, sftp.read_dir(dir)).await {
        Ok(Ok(entries)) => entries,
        Ok(Err(e)) => {
            tracing::debug!(dir, error = %e, "temp sweep: cannot list directory");
            return 0;
        }
        Err(_) => return 0,
    };
    let sep = if dir.ends_with(['/', '\\']) { "" } else { "/" };
    let mut removed = 0;
    for entry in entries {
        let name = entry.file_name();
        if !is_stale_temp(&name, entry.metadata().mtime, now, STALE_TEMP_SECS) {
            continue;
        }
        let path = format!("{dir}{sep}{name}");
        match tokio::time::timeout(timeout, sftp.remove_file(&path)).await {
            Ok(Ok(())) => {
                tracing::info!(path, "removed abandoned upload temp file");
                removed += 1;
            }
            Ok(Err(e)) => tracing::debug!(path, error = %e, "temp sweep: remove failed"),
            Err(_) => tracing::debug!(path, "temp sweep: remove timed out"),
        }
    }
    removed
}

/// `posix-rename@openssh.com`: a rename that atomically replaces an existing
/// target (plain SFTP v3 rename refuses one on OpenSSH).
const POSIX_RENAME: &str = "posix-rename@openssh.com";

/// A second, raw SFTP channel used only for `posix-rename@openssh.com`:
/// russh-sftp's `SftpSession` (2.1 through 3.0) cannot send extended
/// requests, so replacing a file would otherwise need remove-then-rename (B65).
pub struct RenameChannel {
    raw: RawSftpSession,
}

impl RenameChannel {
    /// Rename `from` over `to`, replacing `to` atomically if it exists.
    async fn replace(&self, from: &str, to: &str) -> Result<()> {
        match self
            .raw
            .extended(POSIX_RENAME, posix_rename_payload(from, to))
            .await
        {
            Ok(Packet::Status(s)) if s.status_code == StatusCode::Ok => Ok(()),
            Ok(Packet::Status(s)) => {
                anyhow::bail!(
                    "{POSIX_RENAME} failed: {:?} {}",
                    s.status_code,
                    s.error_message
                )
            }
            Ok(_) => anyhow::bail!("{POSIX_RENAME}: unexpected reply"),
            Err(e) => Err(anyhow::Error::new(e).context(format!("{POSIX_RENAME} failed"))),
        }
    }
}

/// Extended-request payload: SSH strings `oldpath`, `newpath`.
fn posix_rename_payload(from: &str, to: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity(8 + from.len() + to.len());
    for s in [from, to] {
        data.extend_from_slice(&(s.len() as u32).to_be_bytes());
        data.extend_from_slice(s.as_bytes());
    }
    data
}

/// Open a [`RenameChannel`] when the server advertises `posix-rename@openssh.com`
/// (`Ok(None)` otherwise).
pub(crate) async fn open_rename_channel(
    handle: &Handle<SshHandler>,
) -> Result<Option<RenameChannel>> {
    let channel = handle
        .channel_open_session()
        .await
        .context("Failed to open SFTP rename channel")?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .context("Failed to request SFTP subsystem")?;
    let raw = RawSftpSession::new(channel.into_stream());
    let version = raw.init().await.context("SFTP init failed")?;
    let supported = version
        .extensions
        .get(POSIX_RENAME)
        .is_some_and(|v| v == "1");
    Ok(supported.then_some(RenameChannel { raw }))
}

/// Upload a local file to a remote path via SFTP using streaming I/O.
/// The `remote_path` may start with `~` (expanded using `home_dir`).
///
/// Writes to a sibling temp file and renames it over the destination only
/// after the data and the remote close succeeded, so an interrupted upload
/// leaves the existing file intact. With a [`RenameChannel`] the rename
/// replaces the destination atomically; without one (server lacks
/// `posix-rename@openssh.com`) an existing destination is removed first,
/// leaving a brief gap. `timeout` bounds each step, not the whole transfer.
pub async fn upload(
    sftp: &SftpSession,
    rename: Option<&RenameChannel>,
    local_path: &Path,
    remote_path: &str,
    home_dir: &str,
    timeout: Duration,
) -> Result<()> {
    let resolved = resolve_remote_path(remote_path, home_dir);
    let tmp = temp_sibling(&resolved);
    let t = timeout;
    if let Some(parent) = std::path::Path::new(&resolved).parent() {
        if parent != std::path::Path::new("") {
            tokio::time::timeout(t, mkdir_p_sftp(sftp, parent))
                .await
                .context("SFTP mkdir timed out")??;
        }
    }
    let mut local_file = tokio::fs::File::open(local_path)
        .await
        .with_context(|| format!("Cannot open {} for read", local_path.display()))?;
    let mut remote_file = tokio::time::timeout(t, sftp.create(&tmp))
        .await
        .context("SFTP upload open timed out")?
        .with_context(|| format!("SFTP upload open failed for {tmp}"))?;
    let sent = async {
        copy_idle(&mut local_file, &mut remote_file, t).await?;
        step(t, "SFTP flush", remote_file.flush()).await?;
        // Close explicitly: russh-sftp's Drop close is fire-and-forget.
        step(t, "SFTP close", remote_file.shutdown()).await?;
        if let Some(rc) = rename {
            return tokio::time::timeout(t, rc.replace(&tmp, &resolved))
                .await
                .context("SFTP rename timed out")?;
        }
        // SFTP v3 rename refuses an existing target on OpenSSH; retry after
        // removing it (brief gap, but never a truncated file).
        if tokio::time::timeout(t, sftp.rename(&tmp, &resolved))
            .await
            .ok()
            .and_then(|r| r.ok())
            .is_none()
        {
            let _ = tokio::time::timeout(t, sftp.remove_file(&resolved)).await;
            tokio::time::timeout(t, sftp.rename(&tmp, &resolved))
                .await
                .context("SFTP rename timed out")??;
        }
        anyhow::Ok(())
    }
    .await;
    if let Err(e) = sent {
        let _ = tokio::time::timeout(t, sftp.remove_file(&tmp)).await;
        return Err(e.context(format!("SFTP upload failed for {resolved}")));
    }
    Ok(())
}

/// Removes a local temp file on drop unless disarmed (covers cancellation).
struct TempGuard(Option<std::path::PathBuf>);
impl Drop for TempGuard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Stream `src` into a sibling temp of `dest`, then rename it into place.
/// On any error or cancellation `dest` is untouched and the temp is removed.
async fn write_local_atomic<R>(src: &mut R, dest: &Path, idle: Duration) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = std::path::PathBuf::from(temp_sibling(&dest.to_string_lossy()));
    let mut guard = TempGuard(Some(tmp.clone()));
    let mut f = tokio::fs::File::create(&tmp)
        .await
        .with_context(|| format!("Cannot open {} for write", tmp.display()))?;
    copy_idle(src, &mut f, idle).await?;
    step(idle, "flush", f.flush()).await?;
    drop(f);
    tokio::fs::rename(&tmp, dest)
        .await
        .with_context(|| format!("Cannot replace {}", dest.display()))?;
    guard.0 = None;
    Ok(())
}

/// Download a remote file to a local path via SFTP using streaming I/O.
/// The `remote_path` may start with `~` (expanded using `home_dir`).
///
/// Writes a sibling temp file and renames it into place, so an interrupted
/// download leaves the existing local file intact. `timeout` bounds each
/// step, not the whole transfer.
pub async fn download(
    sftp: &SftpSession,
    remote_path: &str,
    local_path: &Path,
    home_dir: &str,
    timeout: Duration,
) -> Result<()> {
    let resolved = resolve_remote_path(remote_path, home_dir);
    let mut remote_file = tokio::time::timeout(timeout, sftp.open(&resolved))
        .await
        .context("SFTP download open timed out")?
        .with_context(|| format!("SFTP download open failed for {}", resolved))?;
    write_local_atomic(&mut remote_file, local_path, timeout)
        .await
        .with_context(|| format!("SFTP download failed for {}", resolved))?;
    let _ = remote_file.shutdown().await; // read handle; nothing to lose
    Ok(())
}

/// Recursively create directories on the remote. An existing directory is
/// fine; any other failure (permission denied, a file in the way) is
/// returned instead of surfacing later as a confusing upload error.
async fn mkdir_p_sftp(sftp: &SftpSession, path: &Path) -> Result<()> {
    let path_str = path.to_string_lossy();
    let parts: Vec<&str> = path_str.split('/').filter(|s| !s.is_empty()).collect();

    let mut current = if path_str.starts_with('/') {
        "/".to_string()
    } else {
        String::new()
    };

    for part in &parts {
        if !current.is_empty() && !current.ends_with('/') {
            current.push('/');
        }
        current.push_str(part);
        if let Err(e) = sftp.create_dir(&current).await {
            // OpenSSH answers a bare FAILURE for "already exists", so only a
            // path that is not a directory afterwards is an error.
            let is_dir = sftp
                .metadata(&current)
                .await
                .is_ok_and(|m| m.file_type().is_dir());
            if !is_dir {
                return Err(
                    anyhow::Error::new(e).context(format!("SFTP mkdir failed for {current}"))
                );
            }
        }
    }
    Ok(())
}

/// SFTP probe: attempt to write and delete a sentinel file.
/// Returns Ok(()) if SFTP is available, Err otherwise.
pub async fn sftp_probe(sftp: &SftpSession, home_dir: &str, timeout: Duration) -> Result<()> {
    tokio::time::timeout(timeout, async {
        let probe_path = format!("{}/.sshi_probe", home_dir);
        sftp.create(&probe_path)
            .await
            .context("SFTP probe create failed")?
            .write_all(b"0")
            .await
            .context("SFTP probe write failed")?;
        if let Err(e) = sftp.remove_file(&probe_path).await {
            tracing::debug!("SFTP probe cleanup failed for {}: {}", probe_path, e);
        }
        Ok(())
    })
    .await
    .context("SFTP probe timed out")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_path_expands_tilde() {
        assert_eq!(
            resolve_remote_path("~/.config/app.toml", "/home/alice"),
            "/home/alice/.config/app.toml"
        );
    }

    #[test]
    fn test_resolve_path_no_tilde() {
        assert_eq!(
            resolve_remote_path("/etc/hosts", "/home/alice"),
            "/etc/hosts"
        );
    }

    #[test]
    fn test_resolve_path_tilde_only() {
        assert_eq!(resolve_remote_path("~", "/home/alice"), "/home/alice");
    }

    /// Regression for G5: `upload`/`download` previously bailed at the 64 MB
    /// `MAX_SFTP_FILE_SIZE` cap; the streaming rewrite streams arbitrary
    /// sizes through `tokio::io::copy` without a size check. This test
    /// exercises the streaming primitive at >64 MB so a future regression
    /// to whole-file buffering would either OOM or hit a re-introduced cap.
    ///
    /// Cannot hit the real `upload`/`download` paths (they require a live
    /// SFTP server), but verifies the underlying streaming pattern copies
    /// >64 MB end-to-end without size limits.
    #[tokio::test]
    async fn streaming_copy_handles_file_larger_than_legacy_64mb_cap() {
        // 64 MiB + 1 byte — one byte past the old MAX_SFTP_FILE_SIZE cap.
        const SIZE: usize = 64 * 1024 * 1024 + 1;
        // Sparse-fill: write first + last byte only; the OS lazy-allocates
        // the middle. Keeps RSS low while still exercising >64MB logical size.
        let mut src = vec![0u8; SIZE];
        src[0] = 0xAA;
        src[SIZE - 1] = 0xBB;
        let mut reader = &src[..];

        let mut sink = Vec::with_capacity(SIZE);
        let copied = tokio::io::copy(&mut reader, &mut sink).await.unwrap();
        assert_eq!(copied, SIZE as u64, "tokio::io::copy must stream all bytes");
        assert_eq!(sink.len(), SIZE);
        assert_eq!(sink[0], 0xAA);
        assert_eq!(sink[SIZE - 1], 0xBB);
    }

    #[test]
    fn temp_sibling_stays_in_same_dir() {
        let pid = std::process::id();
        assert!(temp_sibling("/a/b/f.txt").starts_with(&format!("/a/b/.f.txt.sshi-tmp.{pid}-")));
        assert!(temp_sibling("f").starts_with(&format!(".f.sshi-tmp.{pid}-")));
        assert!(temp_sibling("C:\\x\\f").starts_with(&format!("C:\\x\\.f.sshi-tmp.{pid}-")));
    }

    /// B77: two uploads of one path in one process never share a temp file.
    #[test]
    fn temp_sibling_is_unique_per_upload() {
        let a = temp_sibling("/shared/home/f.bin");
        let b = temp_sibling("/shared/home/f.bin");
        assert_ne!(a, b);
        let name = a.rsplit('/').next().unwrap();
        assert!(
            !is_stale_temp(name, Some(0), u64::MAX, 3600),
            "own pid is never stale"
        );
    }

    /// B65: only another process's abandoned temp file matches the sweep.
    #[test]
    fn stale_temp_detection() {
        let now = 10_000;
        let old = Some(1_000);
        let other = format!(".f.txt{TEMP_MARKER}{}-7", std::process::id() + 1);
        assert!(is_stale_temp(&other, old, now, 3600));
        let legacy = format!(".f.txt{TEMP_MARKER}{}", std::process::id() + 1);
        assert!(
            is_stale_temp(&legacy, old, now, 3600),
            "pre-B77 bare-pid names still swept"
        );
        assert!(!is_stale_temp(".f.txt.sshi-tmp.123-x", old, now, 3600));
        // Fresh (a concurrent run may still be writing it) or no mtime: kept.
        assert!(!is_stale_temp(&other, Some(9_000), now, 3600));
        assert!(!is_stale_temp(&other, None, now, 3600));
        // Our own pid, non-numeric pid, not hidden, or no base name: kept.
        let own = temp_sibling("f.txt");
        assert!(!is_stale_temp(&own, old, now, 3600));
        assert!(!is_stale_temp(".f.txt.sshi-tmp.12x", old, now, 3600));
        assert!(!is_stale_temp("f.txt.sshi-tmp.123", old, now, 3600));
        assert!(!is_stale_temp(".sshi-tmp.123", old, now, 3600));
        assert!(!is_stale_temp("report.txt", old, now, 3600));
    }

    /// B65: the extended request carries two SSH strings, old then new.
    #[test]
    fn posix_rename_payload_is_two_ssh_strings() {
        assert_eq!(
            posix_rename_payload("/a/.f.tmp", "/a/f"),
            [&[0, 0, 0, 9][..], b"/a/.f.tmp", &[0, 0, 0, 4], b"/a/f"].concat()
        );
    }

    /// Reader that yields `ok` chunks, then fails or stalls forever.
    struct Flaky {
        ok: usize,
        stall: bool,
    }
    impl tokio::io::AsyncRead for Flaky {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            use std::task::Poll;
            if self.ok == 0 {
                if self.stall {
                    return Poll::Pending;
                }
                return Poll::Ready(Err(std::io::Error::other("link dropped")));
            }
            self.ok -= 1;
            buf.put_slice(b"NEWDATA");
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn interrupted_local_write_keeps_original_and_no_temp() {
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("f");
        std::fs::write(&dest, "ORIGINAL").unwrap();
        for stall in [false, true] {
            let mut r = Flaky { ok: 3, stall };
            let err = write_local_atomic(&mut r, &dest, Duration::from_millis(50)).await;
            assert!(err.is_err());
            assert_eq!(std::fs::read_to_string(&dest).unwrap(), "ORIGINAL");
            assert_eq!(
                std::fs::read_dir(d.path()).unwrap().count(),
                1,
                "temp left behind"
            );
        }
    }

    #[tokio::test]
    async fn successful_local_write_replaces_dest() {
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("sub/f");
        let mut src: &[u8] = b"hello";
        write_local_atomic(&mut src, &dest, Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "hello");
        assert_eq!(std::fs::read_dir(d.path().join("sub")).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn idle_timeout_is_per_chunk_not_total() {
        // 10 chunks 30ms apart = 300ms total, well over the 100ms idle limit.
        let (mut tx, mut rx) = tokio::io::duplex(64);
        tokio::spawn(async move {
            for _ in 0..10 {
                tokio::time::sleep(Duration::from_millis(30)).await;
                tx.write_all(b"chunk").await.unwrap();
            }
        });
        let mut sink = Vec::new();
        let n = copy_idle(&mut rx, &mut sink, Duration::from_millis(100))
            .await
            .unwrap();
        assert_eq!(n, 50);
        let mut r = Flaky { ok: 0, stall: true };
        let e = copy_idle(&mut r, &mut Vec::new(), Duration::from_millis(100))
            .await
            .unwrap_err();
        assert!(e.to_string().contains("no progress"), "{e}");
    }
}

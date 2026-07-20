//! Test-only [`SessionPool`] implementation that returns canned responses.
//!
//! Used by integration tests in `commands::init::tests` and
//! `commands::sync::integration_tests` to drive `init_core` and the sync
//! phase helpers end-to-end without a live SSH server (audit §2.8 HIGH ×2).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::host::session_pool::{RemoteOutput, SessionPool};

/// Recorded upload call: `(host_ssh_host, local_path, remote_path)`.
pub type UploadCall = (String, PathBuf, String);

/// Recorded download call: `(host_ssh_host, remote_path, local_path)`.
pub type DownloadCall = (String, String, PathBuf);

/// Mock `SessionPool` returning canned responses.
///
/// `exec` responses are keyed by `(host_ssh_host, cmd_substring)` — the
/// first entry whose `host` matches exactly and whose `cmd_substring` is
/// contained in the actual command wins. This lets tests intercept the
/// long batch metadata commands (`build_batch_metadata_cmd` output) without
/// having to reconstruct them byte-for-byte; the test just registers a
/// response keyed on a stable substring like `"---FILE:"`.
///
/// `upload` always succeeds and records the call. `download` writes the
/// bytes registered via [`MockSessionPool::with_download_bytes`] to the
/// local path (or creates an empty file if none registered) and records
/// the call. `upload_error` and `download_error` override the success
/// behaviour for specific `(host, remote_path)` pairs.
pub struct MockSessionPool {
    state: Mutex<State>,
}

struct State {
    reachable: Vec<String>,
    failed: Vec<(String, String)>,
    sftp_failed: Vec<(String, String)>,
    exec_responses: Vec<ExecResponse>,
    download_bytes: HashMap<(String, String), Vec<u8>>,
    upload_errors: HashMap<(String, String), String>,
    download_errors: HashMap<(String, String), String>,
    uploads: Vec<UploadCall>,
    downloads: Vec<DownloadCall>,
}

struct ExecResponse {
    host: String,
    cmd_substring: String,
    output: RemoteOutput,
}

impl MockSessionPool {
    /// New mock with the given reachable host ssh_host names. `failed` and
    /// `sftp_failed` default to empty (i.e. every reachable host is also
    /// SFTP-capable).
    pub fn new(reachable: Vec<String>) -> Self {
        Self {
            state: Mutex::new(State {
                reachable,
                failed: Vec::new(),
                sftp_failed: Vec::new(),
                exec_responses: Vec::new(),
                download_bytes: HashMap::new(),
                upload_errors: HashMap::new(),
                download_errors: HashMap::new(),
                uploads: Vec::new(),
                downloads: Vec::new(),
            }),
        }
    }

    /// Register a host that "failed to connect" with the given error.
    /// Mutates `reachable`/`failed` consistently: the host is removed from
    /// `reachable` if present, and added to `failed`.
    pub fn with_failed_host(self, host: &str, error: &str) -> Self {
        let mut s = self.state.lock().unwrap();
        s.reachable.retain(|h| h != host);
        s.failed.push((host.to_string(), error.to_string()));
        drop(s);
        self
    }

    /// Register a host that connected but failed the SFTP probe.
    pub fn with_sftp_failed_host(self, host: &str, error: &str) -> Self {
        let mut s = self.state.lock().unwrap();
        s.sftp_failed.push((host.to_string(), error.to_string()));
        drop(s);
        self
    }

    /// Register a canned `exec` response. When `exec(host, cmd, _)` is called
    /// and there is a registered entry with matching `host` and a
    /// `cmd_substring` that is a substring of `cmd`, that entry's `output`
    /// is returned. First matching entry wins. If no entry matches, `exec`
    /// returns an `Err` (so unintentional commands surface loudly in tests).
    pub fn with_exec(self, host: &str, cmd_substring: &str, output: RemoteOutput) -> Self {
        let mut s = self.state.lock().unwrap();
        s.exec_responses.push(ExecResponse {
            host: host.to_string(),
            cmd_substring: cmd_substring.to_string(),
            output,
        });
        drop(s);
        self
    }

    /// Register bytes that `download(host, remote_path, _)` should write to
    /// the local file. If no bytes are registered for a `(host, remote_path)`
    /// pair, the download writes an empty file.
    pub fn with_download_bytes(self, host: &str, remote_path: &str, bytes: Vec<u8>) -> Self {
        let mut s = self.state.lock().unwrap();
        s.download_bytes
            .insert((host.to_string(), remote_path.to_string()), bytes);
        drop(s);
        self
    }

    /// Register an error that `upload(host, _, remote_path, _)` should return.
    pub fn with_upload_error(self, host: &str, remote_path: &str, error: &str) -> Self {
        let mut s = self.state.lock().unwrap();
        s.upload_errors.insert(
            (host.to_string(), remote_path.to_string()),
            error.to_string(),
        );
        drop(s);
        self
    }

    /// Register an error that `download(host, remote_path, _)` should return.
    pub fn with_download_error(self, host: &str, remote_path: &str, error: &str) -> Self {
        let mut s = self.state.lock().unwrap();
        s.download_errors.insert(
            (host.to_string(), remote_path.to_string()),
            error.to_string(),
        );
        drop(s);
        self
    }

    /// Snapshot of recorded upload calls, in invocation order.
    pub fn uploads(&self) -> Vec<UploadCall> {
        self.state.lock().unwrap().uploads.clone()
    }

    /// Snapshot of recorded download calls, in invocation order.
    pub fn downloads(&self) -> Vec<DownloadCall> {
        self.state.lock().unwrap().downloads.clone()
    }
}

#[async_trait]
impl SessionPool for MockSessionPool {
    async fn exec(
        &self,
        host_alias: &str,
        cmd: &str,
        _timeout_secs: u64,
    ) -> anyhow::Result<RemoteOutput> {
        let s = self.state.lock().unwrap();
        for resp in &s.exec_responses {
            if resp.host == host_alias && cmd.contains(resp.cmd_substring.as_str()) {
                return Ok(resp.output.clone());
            }
        }
        anyhow::bail!(
            "MockSessionPool: no canned exec response for host='{}', cmd='{}' \
             (register one via with_exec)",
            host_alias,
            cmd
        )
    }

    async fn upload(
        &self,
        host: &crate::config::schema::HostEntry,
        local_path: &Path,
        remote_path: &str,
        _timeout_secs: u64,
    ) -> anyhow::Result<()> {
        let mut s = self.state.lock().unwrap();
        // Always record the call so tests can assert "upload was attempted"
        // even when the mock is configured to return an error.
        s.uploads.push((
            host.ssh_host.clone(),
            local_path.to_path_buf(),
            remote_path.to_string(),
        ));
        if let Some(err) = s
            .upload_errors
            .get(&(host.ssh_host.clone(), remote_path.to_string()))
        {
            anyhow::bail!("{}", err);
        }
        Ok(())
    }

    async fn download(
        &self,
        host: &crate::config::schema::HostEntry,
        remote_path: &str,
        local_path: &Path,
        _timeout_secs: u64,
    ) -> anyhow::Result<()> {
        let mut s = self.state.lock().unwrap();
        // Always record the call so tests can assert "download was attempted"
        // even when the mock is configured to return an error.
        s.downloads.push((
            host.ssh_host.clone(),
            remote_path.to_string(),
            local_path.to_path_buf(),
        ));
        if let Some(err) = s
            .download_errors
            .get(&(host.ssh_host.clone(), remote_path.to_string()))
        {
            anyhow::bail!("{}", err);
        }
        let bytes = s
            .download_bytes
            .get(&(host.ssh_host.clone(), remote_path.to_string()))
            .cloned()
            .unwrap_or_default();
        drop_s_and_write(bytes, local_path)
    }

    fn reachable_hosts(&self) -> Vec<String> {
        self.state.lock().unwrap().reachable.clone()
    }

    fn failed_hosts(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().failed.clone()
    }

    fn sftp_failed_hosts(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().sftp_failed.clone()
    }

    fn sftp_capable_hosts(&self) -> Vec<String> {
        let s = self.state.lock().unwrap();
        let failed: std::collections::HashSet<&str> =
            s.sftp_failed.iter().map(|(n, _)| n.as_str()).collect();
        s.reachable
            .iter()
            .filter(|h| !failed.contains(h.as_str()))
            .cloned()
            .collect()
    }
}

/// Write `bytes` to `local_path`, creating parent dirs as needed. Released
/// from the `State` mutex guard before doing filesystem I/O.
fn drop_s_and_write(bytes: Vec<u8>, local_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = local_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(local_path, &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::HostEntry;

    fn host_entry(ssh_host: &str) -> HostEntry {
        HostEntry::placeholder(ssh_host, ssh_host)
    }

    #[tokio::test]
    async fn exec_returns_registered_response_on_substring_match() {
        let mock = MockSessionPool::new(vec!["h1".into()]).with_exec(
            "h1",
            "PSVersionTable",
            RemoteOutput {
                stdout: "7\n".into(),
                stderr: String::new(),
                exit_code: Some(0),
                success: true,
            },
        );
        let out = mock
            .exec("h1", "$PSVersionTable.PSVersion.Major", 1)
            .await
            .unwrap();
        assert!(out.success);
        assert_eq!(out.stdout.trim(), "7");
    }

    #[tokio::test]
    async fn exec_errors_when_no_response_registered() {
        let mock = MockSessionPool::new(vec!["h1".into()]);
        let err = mock.exec("h1", "anything", 1).await.unwrap_err();
        assert!(err.to_string().contains("no canned exec response"));
    }

    #[tokio::test]
    async fn upload_records_call_and_succeeds_by_default() {
        let mock = MockSessionPool::new(vec!["h1".into()]);
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("src.txt");
        std::fs::write(&local, b"x").unwrap();
        let h = host_entry("h1");
        mock.upload(&h, &local, "/remote/path", 1).await.unwrap();
        let ups = mock.uploads();
        assert_eq!(ups.len(), 1);
        assert_eq!(ups[0].0, "h1");
        assert_eq!(ups[0].2, "/remote/path");
    }

    #[tokio::test]
    async fn upload_returns_registered_error() {
        let mock =
            MockSessionPool::new(vec!["h1".into()]).with_upload_error("h1", "/remote/path", "boom");
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("src.txt");
        let h = host_entry("h1");
        let err = mock
            .upload(&h, &local, "/remote/path", 1)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "boom");
    }

    #[tokio::test]
    async fn download_writes_registered_bytes() {
        let mock = MockSessionPool::new(vec!["h1".into()]).with_download_bytes(
            "h1",
            "/remote/file",
            b"hello".to_vec(),
        );
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("out.txt");
        let h = host_entry("h1");
        mock.download(&h, "/remote/file", &local, 1).await.unwrap();
        assert_eq!(std::fs::read(&local).unwrap(), b"hello");
    }

    #[test]
    fn sftp_capable_excludes_failed_from_reachable() {
        let mock = MockSessionPool::new(vec!["a".into(), "b".into(), "c".into()])
            .with_sftp_failed_host("b", "sftp subsystem not found");
        let capable = mock.sftp_capable_hosts();
        assert!(capable.contains(&"a".to_string()));
        assert!(capable.contains(&"c".to_string()));
        assert!(!capable.contains(&"b".to_string()));
    }

    #[test]
    fn with_failed_host_removes_from_reachable() {
        let mock = MockSessionPool::new(vec!["a".into(), "b".into()])
            .with_failed_host("a", "connection refused");
        assert_eq!(mock.reachable_hosts(), vec!["b".to_string()]);
        assert_eq!(mock.failed_hosts().len(), 1);
        assert_eq!(mock.failed_hosts()[0].0, "a");
    }
}

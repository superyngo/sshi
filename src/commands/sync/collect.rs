use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use tokio::sync::Semaphore;

use crate::config::schema::{HostEntry, ShellType};
use crate::host::quote::ps_in_cmd;
use crate::host::session_pool::SessionPool;

use super::super::Context;
use super::types::{
    BatchCollectResult, CollectResult, DirExpandResult, FileInfo, PathSourceMap, RecursiveEntry,
    SingleFileResult,
};

pub(crate) fn collect_sync_paths<'a>(
    ctx: &'a Context,
    hosts: &[Arc<HostEntry>],
    names: &[String],
    positional: &'a [String],
    cli_source: Option<&'a str>,
) -> (Vec<String>, Vec<RecursiveEntry<'a>>, PathSourceMap<'a>) {
    let mut paths: Vec<String> = Vec::new();
    let mut recursive: Vec<RecursiveEntry<'a>> = Vec::new();
    let mut path_sources: PathSourceMap<'a> = HashMap::new();
    let mut seen: HashSet<String> = HashSet::new();

    for entry in ctx.resolve_syncs(names) {
        let effective_source = cli_source.or(entry.source.as_deref());
        if entry.recursive {
            let all_host_names: HashSet<String> = hosts.iter().map(|h| h.name.clone()).collect();
            recursive.push((entry, all_host_names, effective_source));
        } else {
            for p in &entry.paths {
                if seen.insert(p.clone()) {
                    paths.push(p.clone());
                }
                path_sources.entry(p.clone()).or_insert(effective_source);
            }
        }
    }

    for p in positional {
        let tp = to_tilde_path(p);
        if seen.insert(tp.clone()) {
            paths.push(tp.clone());
        }
        path_sources.entry(tp).or_insert(cli_source);
    }

    (paths, recursive, path_sources)
}

pub(crate) fn requested_sync_paths(
    ctx: &Context,
    names: &[String],
    positional: &[String],
) -> Vec<String> {
    let mut set: std::collections::BTreeSet<String> = ctx
        .resolve_syncs(names)
        .iter()
        .flat_map(|e| e.paths.iter().cloned())
        .collect();
    for p in positional {
        set.insert(to_tilde_path(p));
    }
    set.into_iter().collect()
}

/// Largest metadata command (bytes) sent in one exec, per remote shell. A
/// recursive entry can list thousands of files, but the command reaches the
/// remote as a single shell argument: Linux caps one argv string at 128 KiB,
/// Windows caps a `CreateProcess` line at 32 767 and `cmd.exe` at 8 191 chars.
pub(crate) fn batch_cmd_budget(shell: ShellType) -> usize {
    match shell {
        ShellType::Sh => 32 * 1024,
        ShellType::PowerShell => 16 * 1024,
        ShellType::Cmd => 6 * 1024,
    }
}

/// Split `paths` into consecutive chunks whose metadata command fits `budget`
/// bytes (a single path that alone exceeds it still gets its own chunk).
/// Exponential then binary search keeps this O(n log n) command builds' worth.
pub(crate) fn chunk_paths(paths: &[String], shell: ShellType, budget: usize) -> Vec<&[String]> {
    let fits = |chunk: &[String]| build_batch_metadata_cmd(chunk, shell).len() <= budget;
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < paths.len() {
        // `good` always fits (or is the forced single path); `bad` never does.
        let mut good = start + 1;
        let mut bad = None;
        let mut step = 1;
        while bad.is_none() && good < paths.len() {
            let probe = (good + step).min(paths.len());
            if fits(&paths[start..probe]) {
                good = probe;
                step *= 2;
            } else {
                bad = Some(probe);
            }
        }
        if let Some(mut bad) = bad {
            while good + 1 < bad {
                let mid = good + (bad - good) / 2;
                if fits(&paths[start..mid]) {
                    good = mid;
                } else {
                    bad = mid;
                }
            }
        }
        chunks.push(&paths[start..good]);
        start = good;
    }
    chunks
}

/// Collect metadata for every path on every host with one exec per host per
/// chunk ([`chunk_paths`]). A host whose query fails in any chunk is reported
/// in `failed` and contributes no data at all (as when the batch was one exec).
///
/// Each chunk gets `timeout` seconds per path it hashes: the same total bound
/// the per-file queries had, so batching never times out where they did not.
pub(crate) async fn batch_collect_all_metadata(
    hosts: &[Arc<HostEntry>],
    paths: &[String],
    timeout: u64,
    concurrency: usize,
    sessions: &Arc<dyn SessionPool>,
) -> Result<BatchCollectResult> {
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut set = tokio::task::JoinSet::new();

    for host in hosts {
        let sem = semaphore.clone();
        let host = Arc::clone(host);
        let paths = paths.to_vec();
        let sessions = Arc::clone(sessions);

        set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let mut parsed = HashMap::new();
            for chunk in chunk_paths(&paths, host.shell, batch_cmd_budget(host.shell)) {
                let cmd = build_batch_metadata_cmd(chunk, host.shell);
                let chunk_timeout = timeout.saturating_mul(chunk.len() as u64);
                match sessions.exec(&host.ssh_host, &cmd, chunk_timeout).await {
                    Ok(output) if output.success => parsed.extend(parse_batch_metadata_output(
                        &output.stdout,
                        chunk,
                        &host.name,
                    )),
                    Ok(output) => return (host.name.clone(), Err(exec_failure(&output))),
                    Err(e) => return (host.name.clone(), Err(format!("{e:#}"))),
                }
            }
            (host.name.clone(), Ok(parsed))
        });
    }

    let mut per_file: HashMap<String, CollectResult> = HashMap::new();
    for path in paths {
        per_file.insert(
            path.clone(),
            CollectResult {
                found: Vec::new(),
                missing: Vec::new(),
                failed: Vec::new(),
            },
        );
    }

    let mut failed = Vec::new();
    while let Some(joined) = set.join_next().await {
        let (host_name, parsed) = joined.context("task panic")?;
        match parsed {
            Err(e) => failed.push((host_name, e)),
            Ok(parsed) => {
                for (path, single) in parsed {
                    if let Some(collect) = per_file.get_mut(&path) {
                        if let Some(fi) = single.found {
                            collect.found.push(fi);
                        } else if single.is_missing {
                            collect.missing.push(host_name.clone());
                        }
                    }
                }
            }
        }
    }

    Ok(BatchCollectResult { per_file, failed })
}

/// Reason string for a metadata query that ran but exited non-zero.
fn exec_failure(output: &crate::host::session_pool::RemoteOutput) -> String {
    let code = output
        .exit_code
        .map_or_else(|| "no exit status".to_string(), |c| format!("exit {c}"));
    match output.stderr.trim() {
        "" => code,
        err => format!("{code}: {}", err.lines().next().unwrap_or(err)),
    }
}

pub(crate) fn union_dir_expansions(
    host_results: Vec<HashMap<String, DirExpandResult>>,
) -> HashMap<String, Vec<String>> {
    let mut dirs: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen_per_dir: HashMap<String, HashSet<String>> = HashMap::new();
    for expansions in host_results {
        for (path, result) in expansions {
            if let DirExpandResult::Directory(files) = result {
                let entry = dirs.entry(path.clone()).or_default();
                let seen = seen_per_dir.entry(path).or_default();
                for f in files {
                    if seen.insert(f.clone()) {
                        entry.push(f);
                    }
                }
            }
        }
    }
    dirs
}

pub(crate) async fn expand_directory_paths(
    source_host: &HostEntry,
    paths: &[String],
    recursive: bool,
    timeout: u64,
    sessions: &Arc<dyn SessionPool>,
) -> Result<HashMap<String, DirExpandResult>> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }

    let cmd = build_dir_expand_cmd(paths, recursive, source_host.shell);
    let output = sessions.exec(&source_host.ssh_host, &cmd, timeout).await?;

    if !output.success {
        tracing::warn!(
            host = %source_host.name,
            stderr = %output.stderr,
            "Directory expansion command failed"
        );
        let mut result = HashMap::new();
        for p in paths {
            result.insert(p.clone(), DirExpandResult::Missing);
        }
        return Ok(result);
    }

    Ok(parse_dir_expand_output(&output.stdout, paths))
}

/// sh-quoted remote path via the shared quoting layer (`host::quote`).
fn sh_quote_path(p: &str) -> String {
    crate::host::quote::quote_path(ShellType::Sh, p).expect("sh quoting is infallible")
}

/// PowerShell-quoted remote path via the shared quoting layer (`host::quote`).
/// Also used inside the `-EncodedCommand` script of the Cmd branches.
fn ps_quote_path(p: &str) -> String {
    crate::host::quote::quote_path(ShellType::PowerShell, p)
        .expect("PowerShell quoting is infallible")
}

pub(crate) fn build_batch_metadata_cmd(paths: &[String], shell: ShellType) -> String {
    match shell {
        ShellType::PowerShell => {
            let expanded: Vec<String> = paths.iter().map(|p| ps_quote_path(p)).collect();
            format!(
                "foreach ($f in @({files})) {{ \
                 \"---FILE:$f\"; \
                 $i=Get-Item $f -ErrorAction SilentlyContinue; \
                 if ($i) {{ \
                   [int64](($i.LastWriteTimeUtc-[datetime]\"1970-01-01\").TotalSeconds), $i.Length -join \" \"; \
                   $h=Get-FileHash $f -Algorithm SHA256 -ErrorAction SilentlyContinue; if ($h) {{ $h.Hash.ToLower() }} else {{ \"NOHASH\" }} \
                 }} else {{ \"MISSING\" }} \
                 }}",
                files = expanded.join(",")
            )
        }
        ShellType::Sh => {
            let expanded: Vec<String> = paths.iter().map(|p| sh_quote_path(p)).collect();
            format!(
                "for f in {files}; do \
                 echo \"---FILE:$f\"; \
                 stat -c '%Y %s' \"$f\" 2>/dev/null || stat -f '%m %z' \"$f\" 2>/dev/null || echo \"MISSING\"; \
                 (sha256sum \"$f\" 2>/dev/null || shasum -a 256 \"$f\" 2>/dev/null) || echo \"NOHASH\"; \
                 done",
                files = expanded.join(" ")
            )
        }
        ShellType::Cmd => {
            let expanded: Vec<String> = paths
                .iter()
                .map(|p| ps_quote_path(&p.replace('/', "\\")))
                .collect();
            ps_in_cmd(&format!(
                "foreach ($f in @({files})){{ \
                   '---FILE:' + $f; \
                   $i=Get-Item $f -ErrorAction SilentlyContinue; \
                   if ($i) {{ \
                     [int64](($i.LastWriteTimeUtc-[datetime]'1970-01-01').TotalSeconds), $i.Length -join ' '; \
                     $h=Get-FileHash $f -Algorithm SHA256 -ErrorAction SilentlyContinue; if ($h) {{ $h.Hash.ToLower() }} else {{ \"NOHASH\" }} \
                   }} else {{ 'MISSING' }} \
                 }}",
                files = expanded.join(",")
            ))
        }
    }
}

pub(crate) fn parse_batch_metadata_output(
    output: &str,
    paths: &[String],
    host_name: &str,
) -> HashMap<String, SingleFileResult> {
    let mut result = HashMap::new();

    let blocks: Vec<&str> = output.split("---FILE:").collect();
    let file_blocks = if blocks.len() > 1 {
        &blocks[1..]
    } else {
        return result;
    };

    for (i, block) in file_blocks.iter().enumerate() {
        if i >= paths.len() {
            break;
        }
        let original_path = &paths[i];
        let lines: Vec<&str> = block.lines().collect();

        if lines.len() < 2 {
            result.insert(
                original_path.clone(),
                SingleFileResult {
                    found: None,
                    is_missing: true,
                },
            );
            continue;
        }

        let data_line = lines[1].trim();
        if data_line == "MISSING" {
            result.insert(
                original_path.clone(),
                SingleFileResult {
                    found: None,
                    is_missing: true,
                },
            );
            continue;
        }

        let stat_parts: Vec<&str> = data_line.split_whitespace().collect();
        let mtime: i64 = stat_parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);

        let hash = if lines.len() > 2 {
            let hash_line = lines[2].trim();
            if hash_line == "NOHASH" {
                String::new()
            } else {
                hash_line
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string()
            }
        } else {
            String::new()
        };

        result.insert(
            original_path.clone(),
            SingleFileResult {
                found: Some(FileInfo {
                    host: host_name.to_string(),
                    mtime,
                    hash,
                }),
                is_missing: false,
            },
        );
    }

    result
}

pub(crate) fn build_dir_expand_cmd(paths: &[String], recursive: bool, shell: ShellType) -> String {
    match shell {
        ShellType::PowerShell => {
            let expanded: Vec<String> = paths.iter().map(|p| ps_quote_path(p)).collect();
            let recurse_flag = if recursive { " -Recurse" } else { "" };
            format!(
                "$h=$HOME; \
                 foreach ($p in @({files})) {{ \
                   $orig=$p -replace [regex]::Escape($h),'~'; \
                   \"---PATH:$orig\"; \
                   if (Test-Path $p -PathType Container) {{ \
                     \"DIR\"; \
                     Get-ChildItem $p -File{recurse} | ForEach-Object {{ \
                       $_.FullName -replace [regex]::Escape($h),'~' \
                     }} \
                   }} elseif (Test-Path $p) {{ \"FILE\" }} \
                   else {{ \"MISSING\" }} \
                 }}",
                files = expanded.join(","),
                recurse = recurse_flag
            )
        }
        ShellType::Sh => {
            let expanded: Vec<String> = paths.iter().map(|p| sh_quote_path(p)).collect();
            let depth_flag = if recursive { "" } else { " -maxdepth 1" };
            format!(
                "for p in {files}; do \
                   orig=$(echo \"$p\" | sed \"s|^$HOME/|~/|;s|^$HOME$|~|\"); \
                   echo \"---PATH:$orig\"; \
                   if [ -d \"$p\" ]; then \
                     echo \"DIR\"; \
                     find -L \"$p\"{depth} -type f 2>/dev/null | sed \"s|^$HOME/|~/|\" | sort; \
                   elif [ -e \"$p\" ]; then \
                     echo \"FILE\"; \
                   else \
                     echo \"MISSING\"; \
                   fi; \
                 done",
                files = expanded.join(" "),
                depth = depth_flag
            )
        }
        ShellType::Cmd => {
            let expanded: Vec<String> = paths
                .iter()
                .map(|p| ps_quote_path(&p.replace('/', "\\")))
                .collect();
            let recurse_flag = if recursive { " -Recurse" } else { "" };
            ps_in_cmd(&format!(
                "foreach ($p in @({files})) {{ \
                   '---PATH:' + $p; \
                   if (Test-Path $p -PathType Container) {{ \
                     'DIR'; \
                     Get-ChildItem $p -File{recurse} | ForEach-Object {{ $_.FullName }} \
                   }} elseif (Test-Path $p) {{ 'FILE' }} \
                   else {{ 'MISSING' }} \
                 }}",
                files = expanded.join(","),
                recurse = recurse_flag
            ))
        }
    }
}

pub(crate) fn parse_dir_expand_output(
    output: &str,
    paths: &[String],
) -> HashMap<String, DirExpandResult> {
    let mut result = HashMap::new();

    let blocks: Vec<&str> = output.split("---PATH:").collect();
    let file_blocks = if blocks.len() > 1 {
        &blocks[1..]
    } else {
        return result;
    };

    for (i, block) in file_blocks.iter().enumerate() {
        if i >= paths.len() {
            break;
        }
        let original_path = &paths[i];
        let lines: Vec<&str> = block.lines().collect();

        if lines.len() < 2 {
            result.insert(original_path.clone(), DirExpandResult::Missing);
            continue;
        }

        let type_marker = lines[1].trim();
        match type_marker {
            "DIR" => {
                let files: Vec<String> = lines[2..]
                    .iter()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty())
                    .map(|l| l.to_string())
                    .collect();
                result.insert(original_path.clone(), DirExpandResult::Directory(files));
            }
            "FILE" => {
                result.insert(original_path.clone(), DirExpandResult::File);
            }
            _ => {
                result.insert(original_path.clone(), DirExpandResult::Missing);
            }
        }
    }

    result
}

fn to_tilde_path(path: &str) -> String {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok();
    if let Some(ref home) = home {
        if !home.is_empty() {
            if path == home.as_str() {
                return "~".to_string();
            }
            for sep in &["/", "\\"] {
                let prefix = format!("{}{}", home, sep);
                if let Some(rest) = path.strip_prefix(&prefix) {
                    return format!("~/{}", rest);
                }
            }
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_union_dir_expansions_10k_files_under_1s() {
        let start = std::time::Instant::now();
        const N: usize = 10_000;

        let files_a: Vec<String> = (0..N).map(|i| format!("/srv/file_{i}.bin")).collect();
        let files_b: Vec<String> = (0..N).map(|i| format!("/srv/file_{}.bin", N + i)).collect();
        let dup: Vec<String> = files_a[..100].to_vec();

        let mut host_a: HashMap<String, DirExpandResult> = HashMap::new();
        host_a.insert(
            "/srv".to_string(),
            DirExpandResult::Directory(files_a.clone()),
        );

        let mut host_b: HashMap<String, DirExpandResult> = HashMap::new();
        host_b.insert("/srv".to_string(), DirExpandResult::Directory(files_b));

        let mut host_c: HashMap<String, DirExpandResult> = HashMap::new();
        host_c.insert("/srv".to_string(), DirExpandResult::Directory(dup));

        let merged = union_dir_expansions(vec![host_a, host_b, host_c]);

        let entry = merged.get("/srv").expect("merged /srv entry");
        assert_eq!(entry.len(), 2 * N);
        assert!(entry.iter().all(|f| !f.is_empty()));

        let elapsed = start.elapsed();
        assert!(
            elapsed.as_secs() < 1,
            "10k-file expansion took {:?}, expected <1s",
            elapsed,
        );
    }

    #[test]
    fn test_powershell_path_injection_is_neutralized() {
        let paths = vec!["$(echo PWNED)".to_string()];
        let cmd = build_batch_metadata_cmd(&paths, ShellType::PowerShell);

        assert!(
            cmd.contains("'$(echo PWNED)'"),
            "expected `$(echo PWNED)` to be single-quoted in cmd: {cmd}"
        );
        assert!(
            !cmd.contains("\"$(echo PWNED)\""),
            "found double-quoted malicious token in cmd: {cmd}"
        );
    }

    /// B34: a recursive entry's thousands of paths are split so every exec'd
    /// command fits the per-shell budget, and every path is still collected.
    #[tokio::test]
    async fn batch_collect_chunks_commands_under_budget() {
        use crate::host::session_pool::RemoteOutput;
        use crate::host::session_pool_mock::MockSessionPool;
        let paths: Vec<String> = (0..5000)
            .map(|i| format!("~/very/long/nested/directory/structure/file_{i:05}.txt"))
            .collect();
        for shell in [ShellType::Sh, ShellType::PowerShell, ShellType::Cmd] {
            let budget = batch_cmd_budget(shell);
            let chunks = chunk_paths(&paths, shell, budget);
            assert!(chunks.len() > 1, "{shell:?}");
            assert_eq!(chunks.iter().map(|c| c.len()).sum::<usize>(), paths.len());
            for chunk in &chunks {
                assert!(build_batch_metadata_cmd(chunk, shell).len() <= budget);
            }
            // Greedy: adding the next path to a chunk would overflow it.
            let first = chunks[0].len();
            assert!(build_batch_metadata_cmd(&paths[..first + 1], shell).len() > budget);
        }
        // A single path larger than the budget still gets its own chunk.
        let huge = vec!["x".repeat(100), "y".to_string()];
        let chunks = chunk_paths(&huge, ShellType::Sh, 10);
        assert_eq!(chunks.len(), 2);

        let ok = RemoteOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        };
        let mock =
            Arc::new(MockSessionPool::new(vec!["h1".into()]).with_exec("h1", "---FILE:", ok));
        let sessions: Arc<dyn SessionPool> = mock.clone();
        let hosts = vec![Arc::new(HostEntry::placeholder("h1", "h1"))];
        let result = batch_collect_all_metadata(&hosts, &paths, 5, 2, &sessions)
            .await
            .unwrap();
        assert_eq!(result.per_file.len(), paths.len());
        let calls = mock.exec_calls();
        let expected = chunk_paths(&paths, ShellType::Sh, batch_cmd_budget(ShellType::Sh)).len();
        assert_eq!(calls.len(), expected);
        assert!(calls
            .iter()
            .all(|(_, cmd)| cmd.len() <= batch_cmd_budget(ShellType::Sh)));
    }
}

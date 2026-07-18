use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Semaphore;

use crate::config::schema::{HostEntry, ShellType};
use crate::host::session_pool::RusshSessionPool;

use super::super::Context;
use super::types::{
    BatchCollectResult, CollectResult, DirExpandResult, FileInfo, HostPathMap, PathSourceMap,
    RecursiveEntry, SingleFileResult,
};

pub(crate) fn collect_sync_paths<'a>(
    ctx: &'a Context,
    hosts: &[&HostEntry],
    names: &[String],
    positional: &'a [String],
    cli_source: Option<&'a str>,
) -> (
    Vec<String>,
    Vec<RecursiveEntry<'a>>,
    Option<HostPathMap>,
    PathSourceMap<'a>,
) {
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

    (paths, recursive, None, path_sources)
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

pub(crate) fn scope_collect_result(
    collect: &CollectResult,
    path: &str,
    host_applicable_paths: &Option<HostPathMap>,
) -> (Vec<FileInfo>, Vec<String>) {
    match host_applicable_paths {
        Some(map) => {
            let found: Vec<FileInfo> = collect
                .found
                .iter()
                .filter(|fi| map.get(&fi.host).is_some_and(|paths| paths.contains(path)))
                .cloned()
                .collect();
            let missing: Vec<String> = collect
                .missing
                .iter()
                .filter(|host| map.get(*host).is_some_and(|paths| paths.contains(path)))
                .cloned()
                .collect();
            (found, missing)
        }
        None => (collect.found.clone(), collect.missing.clone()),
    }
}

pub(crate) async fn collect_file_metadata(
    hosts: &[&HostEntry],
    path: &str,
    timeout: u64,
    concurrency: usize,
    sessions: Arc<RusshSessionPool>,
) -> Result<CollectResult> {
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::new();

    for host in hosts {
        let sem = semaphore.clone();
        let host = (*host).clone();
        let file_path = path.to_string();
        let sessions = Arc::clone(&sessions);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();

            let cmd = match host.shell {
                ShellType::PowerShell => {
                    let ps_path = if let Some(stripped) = file_path.strip_prefix("~/") {
                        let rest = stripped.replace('/', "\\").replace('\'', "''");
                        format!("\"$HOME\" + '\\{}'", rest)
                    } else {
                        format!("'{}'", file_path.replace('\'', "''"))
                    };
                    format!(
                        "$f={p}; \
                         $i=Get-Item $f -ErrorAction SilentlyContinue; \
                         if ($i) {{ \
                           [int64](($i.LastWriteTimeUtc-[datetime]\"1970-01-01\").TotalSeconds), $i.Length -join \" \"; \
                           (Get-FileHash $f -Algorithm SHA256).Hash.ToLower() \
                         }}",
                        p = ps_path
                    )
                }
                ShellType::Sh => {
                    let escaped = if let Some(stripped) = file_path.strip_prefix("~/") {
                        format!("$HOME/'{}'", stripped.replace('\'', "'\\''"))
                    } else {
                        format!("'{}'", file_path.replace('\'', "'\\''"))
                    };
                    format!(
                        "stat -c '%Y %s' {p} 2>/dev/null || stat -f '%m %z' {p} 2>/dev/null; \
                         (sha256sum {p} 2>/dev/null || shasum -a 256 {p} 2>/dev/null) || true",
                        p = escaped
                    )
                }
                ShellType::Cmd => {
                    let escaped = file_path.replace('\'', "''");
                    format!(
                        "powershell -NoProfile -Command \"\
                         $i=Get-Item '{p}' -ErrorAction SilentlyContinue; \
                         if ($i) {{ \
                           [int64](($i.LastWriteTimeUtc-[datetime]'1970-01-01').TotalSeconds), $i.Length -join ' '; \
                           (Get-FileHash '{p}' -Algorithm SHA256).Hash.ToLower() \
                         }}\"",
                        p = escaped
                    )
                }
            };

            match sessions.exec(&host.ssh_host, &cmd, timeout).await {
                Ok(output) => {
                    let lines: Vec<&str> = output.stdout.lines().collect();
                    let stat_parts: Vec<&str> = lines
                        .first()
                        .map(|l| l.split_whitespace().collect())
                        .unwrap_or_default();
                    let mtime: Option<i64> = stat_parts.first().and_then(|s| s.parse().ok());

                    if let Some(mtime) = mtime {
                        let hash = lines
                            .get(1)
                            .and_then(|l| l.split_whitespace().next())
                            .unwrap_or("")
                            .to_string();
                        (host.name.clone(), Some(FileInfo {
                            host: host.name.clone(),
                            mtime,
                            hash,
                        }), false)
                    } else {
                        (host.name.clone(), None, true)
                    }
                }
                Err(_) => {
                    (host.name.clone(), None, false)
                }
            }
        }));
    }

    let mut found = Vec::new();
    let mut missing = Vec::new();
    for handle in handles {
        let (host_name, info, is_missing) = handle.await?;
        if let Some(fi) = info {
            found.push(fi);
        } else if is_missing {
            missing.push(host_name);
        }
    }

    Ok(CollectResult { found, missing })
}

pub(crate) async fn batch_collect_all_metadata(
    hosts: &[&HostEntry],
    paths: &[String],
    timeout: u64,
    concurrency: usize,
    sessions: &Arc<RusshSessionPool>,
) -> Result<BatchCollectResult> {
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut handles = Vec::new();

    for host in hosts {
        let sem = semaphore.clone();
        let host = (*host).clone();
        let paths = paths.to_vec();
        let cmd = build_batch_metadata_cmd(&paths, host.shell);
        let sessions = Arc::clone(sessions);

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let result = sessions.exec(&host.ssh_host, &cmd, timeout).await;
            match result {
                Ok(output) if output.success => {
                    let parsed = parse_batch_metadata_output(&output.stdout, &paths, &host.name);
                    (host.name.clone(), Some(parsed), false)
                }
                _ => (host.name.clone(), None, true),
            }
        }));
    }

    let mut per_file: HashMap<String, CollectResult> = HashMap::new();
    for path in paths {
        per_file.insert(
            path.clone(),
            CollectResult {
                found: Vec::new(),
                missing: Vec::new(),
            },
        );
    }

    for handle in handles {
        let (host_name, parsed_opt, is_unreachable) = handle.await?;
        if is_unreachable {
            continue;
        }
        if let Some(parsed) = parsed_opt {
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

    Ok(BatchCollectResult { per_file })
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
    sessions: &Arc<RusshSessionPool>,
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

pub(crate) fn build_batch_metadata_cmd(paths: &[String], shell: ShellType) -> String {
    match shell {
        ShellType::PowerShell => {
            let expanded: Vec<String> = paths
                .iter()
                .map(|p| {
                    if let Some(stripped) = p.strip_prefix("~/") {
                        let rest = stripped.replace('/', "\\").replace('\'', "''");
                        format!("(\"$HOME\" + '\\{}')", rest)
                    } else {
                        format!("'{}'", p.replace('\'', "''"))
                    }
                })
                .collect();
            format!(
                "foreach ($f in @({files})) {{ \
                 \"---FILE:$f\"; \
                 $i=Get-Item $f -ErrorAction SilentlyContinue; \
                 if ($i) {{ \
                   [int64](($i.LastWriteTimeUtc-[datetime]\"1970-01-01\").TotalSeconds), $i.Length -join \" \"; \
                   (Get-FileHash $f -Algorithm SHA256).Hash.ToLower() \
                 }} else {{ \"MISSING\" }} \
                 }}",
                files = expanded.join(",")
            )
        }
        ShellType::Sh => {
            let expanded: Vec<String> = paths
                .iter()
                .map(|p| {
                    if let Some(stripped) = p.strip_prefix("~/") {
                        format!("$HOME/'{}'", stripped.replace('\'', "'\\''"))
                    } else {
                        format!("'{}'", p.replace('\'', "'\\''"))
                    }
                })
                .collect();
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
                .map(|p| {
                    let p = p.replace('/', "\\").replace('"', "`\"");
                    format!("\"{}\"", p)
                })
                .collect();
            format!(
                "powershell -NoProfile -Command \"\
                 foreach ($f in @({files})){{ \
                   '---FILE:' + $f; \
                   $i=Get-Item $f -ErrorAction SilentlyContinue; \
                   if ($i) {{ \
                     [int64](($i.LastWriteTimeUtc-[datetime]'1970-01-01').TotalSeconds), $i.Length -join ' '; \
                     (Get-FileHash $f -Algorithm SHA256).Hash.ToLower() \
                   }} else {{ 'MISSING' }} \
                 }}\"",
                files = expanded.join(",")
            )
        }
    }
}

#[allow(dead_code)]
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
            let expanded: Vec<String> = paths
                .iter()
                .map(|p| {
                    if let Some(stripped) = p.strip_prefix("~/") {
                        format!("\"$HOME\\{}\"", stripped.replace('/', "\\"))
                    } else {
                        format!("\"{}\"", p)
                    }
                })
                .collect();
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
            let expanded: Vec<String> = paths
                .iter()
                .map(|p| {
                    if let Some(stripped) = p.strip_prefix("~/") {
                        format!("$HOME/'{}'", stripped.replace('\'', "'\\''"))
                    } else {
                        format!("'{}'", p.replace('\'', "'\\''"))
                    }
                })
                .collect();
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
                .map(|p| {
                    let p = p.replace('/', "\\").replace('"', "`\"");
                    format!("\"{}\"", p)
                })
                .collect();
            let recurse_flag = if recursive { " -Recurse" } else { "" };
            format!(
                "powershell -NoProfile -Command \"\
                 foreach ($p in @({files})) {{ \
                   '---PATH:' + $p; \
                   if (Test-Path $p -PathType Container) {{ \
                     'DIR'; \
                     Get-ChildItem $p -File{recurse} | ForEach-Object {{ $_.FullName }} \
                   }} elseif (Test-Path $p) {{ 'FILE' }} \
                   else {{ 'MISSING' }} \
                 }}\"",
                files = expanded.join(","),
                recurse = recurse_flag
            )
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
}

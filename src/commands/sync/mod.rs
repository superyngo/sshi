mod collect;
mod decide;
mod distribute;
mod report;
mod types;

pub(crate) use types::SyncOutputStyle;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Semaphore;

use crate::config::schema::HostEntry;
use crate::host::pool::SshPool;
use crate::host::session_pool::RusshSessionPool;
use crate::output::printer;
use crate::output::summary::SyncSummary;

use super::Context;

use collect::{
    batch_collect_all_metadata, collect_file_metadata, collect_sync_paths, expand_directory_paths,
    requested_sync_paths, scope_collect_result, union_dir_expansions,
};
use decide::{make_decisions, make_decisions_fixed_source};
use distribute::{distribute, distribute_pooled};
use report::build_sync_report;
use types::{DirExpandResult, SyncDecision};

pub async fn run(
    ctx: &Context,
    dry_run: bool,
    positional: &[String],
    names: &[String],
    cli_source: Option<&str>,
    output: &crate::cli::OutputArgs,
) -> Result<()> {
    let report = sync_inner(
        ctx,
        dry_run,
        positional,
        names,
        cli_source,
        SyncOutputStyle::Verbose,
        None,
    )
    .await?;
    crate::output::report::maybe_write_report(
        &report,
        &ctx.mode,
        output.out.as_deref(),
        "sync",
        ctx.config.settings.default_output_format.as_deref(),
    )?;
    Ok(())
}

pub async fn sync_core(
    ctx: &Context,
    positional: &[String],
    names: &[String],
    dry_run: bool,
    cli_source: Option<&str>,
    progress: Option<&(dyn crate::commands::report::ProgressSink + Sync)>,
) -> Result<crate::commands::report::CommandReport> {
    sync_inner(
        ctx,
        dry_run,
        positional,
        names,
        cli_source,
        SyncOutputStyle::Quiet,
        progress,
    )
    .await
}

async fn sync_inner(
    ctx: &Context,
    dry_run: bool,
    positional: &[String],
    names: &[String],
    cli_source: Option<&str>,
    output_style: SyncOutputStyle,
    progress: Option<&(dyn crate::commands::report::ProgressSink + Sync)>,
) -> Result<crate::commands::report::CommandReport> {
    use crate::commands::report::CommandReport;

    let verbose = output_style.is_verbose();
    let executed_at = chrono::Utc::now().to_rfc3339();
    let mode = if positional.is_empty() {
        "config_entries"
    } else {
        "adhoc"
    };
    let push_missing = true;
    let hosts = ctx.resolve_hosts()?;
    let host_names: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();
    let mut host_file_map: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
    let mut summary = SyncSummary::default();

    let requested_paths = requested_sync_paths(ctx, names, positional);

    for host in &hosts {
        host_file_map
            .entry(host.name.clone())
            .or_insert((Vec::new(), Vec::new()));
    }

    if let Some(p) = progress {
        for host in &host_names {
            p.host_started(host);
        }
    }

    let (mut all_paths, recursive_entries, mut host_applicable_paths, mut path_source_map) =
        collect_sync_paths(ctx, &hosts, names, positional, cli_source);
    if all_paths.is_empty() && recursive_entries.is_empty() {
        if verbose {
            if names.is_empty() && positional.is_empty() {
                println!("Nothing to sync. Pass paths and/or -n/--name (named [[sync]] entries).");
            } else {
                println!("No sync paths resolved. Check -n/--name values or pass paths directly.");
            }
        } else {
            tracing::info!("No sync paths resolved from --name entries or positional paths");
        }
        let report = build_sync_report(
            executed_at,
            mode,
            dry_run,
            &host_names,
            &host_file_map,
            &summary,
            &[],
            &requested_paths,
        );
        return Ok(CommandReport::Sync(report));
    }

    if hosts.len() < 2 && (!all_paths.is_empty() || !recursive_entries.is_empty()) {
        if verbose {
            println!("Need at least 2 hosts for sync (found {})", hosts.len());
        } else {
            tracing::info!("Need at least 2 hosts for sync (found {})", hosts.len());
        }
        let report = build_sync_report(
            executed_at,
            mode,
            dry_run,
            &host_names,
            &host_file_map,
            &summary,
            &[],
            &requested_paths,
        );
        return Ok(CommandReport::Sync(report));
    }

    let label = match &ctx.mode {
        super::TargetMode::All => "global".to_string(),
        super::TargetMode::Groups(g) => g.join(", "),
        super::TargetMode::Hosts(h) => h.join(", "),
        super::TargetMode::Shell(s) => s
            .iter()
            .map(|sh| sh.to_string())
            .collect::<Vec<_>>()
            .join(", "),
    };
    if verbose {
        println!("\n── Sync: {} ──", label);
    }

    let (pool, _connected) = SshPool::setup_with_options(
        &hosts,
        ctx.timeout,
        ctx.concurrency(),
        ctx.per_host_concurrency(),
        true,
    )
    .await?;

    for (name, err) in pool.failed_hosts() {
        if verbose {
            printer::print_host_line("unreachable", "error", &format!("{}: {}", name, err));
        } else {
            tracing::warn!(host = %name, error = %err, "unreachable");
        }
        summary.add_host_failure(&name, &err);
    }

    for (name, err) in pool.sftp_failed_hosts() {
        if verbose {
            printer::print_host_line("sftp-failed", "error", &format!("{}: {}", name, err));
        } else {
            tracing::warn!(host = %name, error = %err, "sftp probe failed");
        }
        summary.add_host_failure(&name, &format!("sftp probe failed: {}", err));
    }

    let reachable_hosts = pool.filter_sftp_capable(&hosts);
    if let Some(src) = cli_source {
        if !reachable_hosts.iter().any(|h| h.name == src) {
            pool.shutdown().await;
            anyhow::bail!("Source host '{}' is unreachable or SFTP-incapable", src);
        }
    }

    if reachable_hosts.len() < 2 && !all_paths.is_empty() {
        if verbose {
            println!(
                "Need at least 2 reachable hosts for sync (found {})",
                reachable_hosts.len()
            );
        } else {
            tracing::info!(
                "Need at least 2 reachable hosts for sync (found {})",
                reachable_hosts.len()
            );
        }
        let failed_host_names: Vec<String> = pool
            .failed_hosts()
            .into_iter()
            .map(|(n, _)| n)
            .chain(pool.sftp_failed_hosts().into_iter().map(|(n, _)| n))
            .collect();
        pool.shutdown().await;
        let report = build_sync_report(
            executed_at,
            mode,
            dry_run,
            &host_names,
            &host_file_map,
            &summary,
            &failed_host_names,
            &requested_paths,
        );
        return Ok(CommandReport::Sync(report));
    }

    // Step 3.5: Expand directory paths for entries with a fixed source.
    {
        let source_paths: Vec<(String, &str)> = all_paths
            .iter()
            .filter_map(|p| {
                path_source_map
                    .get(p.as_str())
                    .and_then(|s| s.as_ref())
                    .map(|src| (p.clone(), *src))
            })
            .collect();

        if !source_paths.is_empty() {
            let mut by_source: HashMap<&str, Vec<String>> = HashMap::new();
            for (path, src) in &source_paths {
                by_source.entry(src).or_default().push(path.clone());
            }

            let mut dirs_expanded: HashMap<String, Vec<String>> = HashMap::new();
            let mut dirs_missing: Vec<String> = Vec::new();

            for (src_name, paths_for_src) in &by_source {
                if let Some(source_host) = reachable_hosts.iter().find(|h| h.name == *src_name) {
                    match expand_directory_paths(
                        source_host,
                        paths_for_src,
                        false,
                        ctx.timeout,
                        &pool.session_pool,
                    )
                    .await
                    {
                        Ok(expansions) => {
                            for (path, result) in expansions {
                                match result {
                                    DirExpandResult::Directory(files) => {
                                        dirs_expanded.insert(path, files);
                                    }
                                    DirExpandResult::Missing => {
                                        dirs_missing.push(path);
                                    }
                                    DirExpandResult::File => {}
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                source = %src_name,
                                error = %e,
                                "Failed to expand directories from source"
                            );
                        }
                    }
                }
            }

            if !dirs_expanded.is_empty() || !dirs_missing.is_empty() {
                let mut new_paths = Vec::new();
                let mut new_paths_seen: HashSet<String> = HashSet::new();
                for path in &all_paths {
                    if let Some(expanded_files) = dirs_expanded.get(path) {
                        let src = path_source_map.get(path.as_str()).copied().flatten();
                        for file_path in expanded_files {
                            if new_paths_seen.insert(file_path.clone()) {
                                new_paths.push(file_path.clone());
                                path_source_map.entry(file_path.clone()).or_insert(src);
                            }
                        }
                        if let Some(ref mut host_map) = host_applicable_paths {
                            for (_host, path_set) in host_map.iter_mut() {
                                if path_set.remove(path) {
                                    for file_path in expanded_files {
                                        path_set.insert(file_path.clone());
                                    }
                                }
                            }
                        }
                        if verbose && expanded_files.is_empty() {
                            println!("  {} (empty directory on source, skipping)", path);
                        }
                    } else {
                        new_paths.push(path.clone());
                    }
                }
                all_paths = new_paths;
            }
        }
    }

    // Step 3.6: Expand directory paths for entries with no fixed source.
    {
        let no_source_paths: Vec<String> = all_paths
            .iter()
            .filter(|p| path_source_map.get(p.as_str()).is_none_or(|s| s.is_none()))
            .cloned()
            .collect();

        if !no_source_paths.is_empty() {
            let semaphore = Arc::new(Semaphore::new(ctx.concurrency()));
            let mut handles = Vec::new();

            for host in &reachable_hosts {
                let host = (*host).clone();
                let paths = no_source_paths.clone();
                let sessions = Arc::clone(&pool.session_pool);
                let timeout = ctx.timeout;
                let sem = semaphore.clone();

                handles.push(tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    expand_directory_paths(&host, &paths, false, timeout, &sessions).await
                }));
            }

            let mut host_results: Vec<HashMap<String, DirExpandResult>> = Vec::new();
            for handle in handles {
                match handle.await {
                    Ok(Ok(expansions)) => host_results.push(expansions),
                    Ok(Err(e)) => tracing::warn!(error = %e, "Failed to expand directories"),
                    Err(e) => tracing::warn!(error = %e, "Directory expand task panicked"),
                }
            }

            let dirs_expanded = union_dir_expansions(host_results);

            if !dirs_expanded.is_empty() {
                let mut new_paths = Vec::new();
                let mut new_paths_seen: HashSet<String> = HashSet::new();
                for path in &all_paths {
                    if let Some(expanded_files) = dirs_expanded.get(path) {
                        for file_path in expanded_files {
                            if new_paths_seen.insert(file_path.clone()) {
                                new_paths.push(file_path.clone());
                                path_source_map.entry(file_path.clone()).or_insert(None);
                            }
                        }
                        if let Some(ref mut host_map) = host_applicable_paths {
                            for (_host, path_set) in host_map.iter_mut() {
                                if path_set.remove(path) {
                                    for file_path in expanded_files {
                                        path_set.insert(file_path.clone());
                                    }
                                }
                            }
                        }
                        if verbose && expanded_files.is_empty() {
                            println!("  {} (empty directory on all hosts, skipping)", path);
                        }
                    } else {
                        new_paths.push(path.clone());
                    }
                }
                all_paths = new_paths;
            }
        }
    }

    // Step 4: Batch collect metadata for non-recursive files
    if !all_paths.is_empty() {
        let batch_result = batch_collect_all_metadata(
            &reachable_hosts,
            &all_paths,
            ctx.timeout,
            ctx.concurrency(),
            &pool.session_pool,
        )
        .await?;

        // Step 5: Make decisions per file
        let mut all_decisions: Vec<SyncDecision> = Vec::new();
        for path in &all_paths {
            if let Some(collect) = batch_result.per_file.get(path) {
                let (scoped_found, scoped_missing) =
                    scope_collect_result(collect, path, &host_applicable_paths);

                if scoped_found.is_empty() {
                    if verbose {
                        if !scoped_missing.is_empty() {
                            println!("  {}: file not found on any reachable host", path);
                        } else {
                            println!("  {}: no data collected", path);
                        }
                    } else if !scoped_missing.is_empty() {
                        tracing::debug!(path, "file not found on any reachable host");
                    } else {
                        tracing::debug!(path, "no data collected");
                    }
                    continue;
                }

                let effective_source =
                    cli_source.or_else(|| path_source_map.get(path.as_str()).copied().flatten());
                let decisions = if let Some(src) = effective_source {
                    let (decs, skip_info) = make_decisions_fixed_source(
                        &scoped_found,
                        path,
                        push_missing,
                        &scoped_missing,
                        src,
                    )?;
                    if let Some((source, skipped_path)) = skip_info {
                        if verbose {
                            printer::print_host_line(
                                "skip",
                                &source,
                                &format!("does not have '{}'", skipped_path),
                            );
                        }
                        summary.add_skip_with_reason(
                            &skipped_path,
                            &source,
                            &format!("source '{}' does not have '{}'", source, skipped_path),
                        );
                        continue;
                    }
                    decs
                } else {
                    make_decisions(
                        &scoped_found,
                        &ctx.config.settings.conflict_strategy,
                        path,
                        push_missing,
                        &scoped_missing,
                    )
                };

                if decisions.is_empty() {
                    let hosts_list: Vec<&str> =
                        scoped_found.iter().map(|f| f.host.as_str()).collect();
                    if verbose {
                        println!("  {} (all in sync)", path);
                        printer::print_host_line("passed", "ok", &hosts_list.join(", "));
                    } else {
                        tracing::debug!(path, hosts = %hosts_list.join(", "), "all in sync");
                    }
                    summary.file_in_sync(&hosts_list);
                } else {
                    all_decisions.extend(decisions);
                }
            }
        }

        // Step 6: Distribute all files
        if dry_run {
            for d in &all_decisions {
                let mut all_targets: Vec<&str> =
                    d.synced_hosts.iter().map(|s| s.as_str()).collect();
                all_targets.extend(d.target_hosts.iter().map(|s| s.as_str()));
                if verbose {
                    println!(
                        "  {} → source: {} → targets: [{}] ({})",
                        d.path,
                        d.source_host,
                        all_targets.join(", "),
                        d.reason
                    );
                    if !d.synced_hosts.is_empty() {
                        printer::print_host_line("passed", "ok", &d.synced_hosts.join(", "));
                    }
                } else {
                    tracing::debug!(path = %d.path, source = %d.source_host, "[dry-run] would sync");
                }
                for h in &d.synced_hosts {
                    host_file_map
                        .entry(h.clone())
                        .or_default()
                        .1
                        .push(d.path.clone());
                }
            }
            if verbose && !all_decisions.is_empty() {
                println!("  [dry-run] No changes applied.");
            }
        } else {
            for decision in &all_decisions {
                if verbose {
                    let mut all_targets: Vec<&str> =
                        decision.synced_hosts.iter().map(|s| s.as_str()).collect();
                    all_targets.extend(decision.target_hosts.iter().map(|s| s.as_str()));
                    println!(
                        "  {} → source: {} → targets: [{}] ({})",
                        decision.path,
                        decision.source_host,
                        all_targets.join(", "),
                        decision.reason
                    );
                    if !decision.synced_hosts.is_empty() {
                        printer::print_host_line("passed", "ok", &decision.synced_hosts.join(", "));
                    }
                } else {
                    tracing::debug!(
                        path = %decision.path, source = %decision.source_host,
                        targets = %decision.target_hosts.join(", ")
                    );
                    if !decision.synced_hosts.is_empty() {
                        tracing::debug!(hosts = %decision.synced_hosts.join(", "), "already in sync");
                    }
                }

                match distribute_pooled(
                    &reachable_hosts,
                    decision,
                    ctx.timeout,
                    &pool.limiter,
                    &pool.session_pool,
                )
                .await
                {
                    Ok((succeeded, failed_uploads)) => {
                        if !succeeded.is_empty() {
                            if verbose {
                                printer::print_host_line("synced", "ok", &succeeded.join(", "));
                            } else {
                                tracing::debug!(hosts = %succeeded.join(", "), "synced");
                            }

                            let now = chrono::Utc::now().timestamp();
                            for target in &succeeded {
                                if let Err(e) = ctx.db.execute(
                                    "INSERT INTO sync_state (sync_group, host, path, mtime, size_bytes, blake3, synced_at) \
                                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                                     ON CONFLICT(sync_group, host, path) DO UPDATE SET mtime=?4, size_bytes=?5, blake3=?6, synced_at=?7",
                                    rusqlite::params![
                                        label, target, decision.path,
                                        0i64, 0i64, "", now
                                    ],
                                ) {
                                    tracing::warn!(error = %e, "failed to record operation_log entry");
                                }
                            }

                            if let Err(e) = ctx.db.execute(
                                "INSERT INTO operation_log (timestamp, command, host, action, status, duration_ms) \
                                 VALUES (?1, 'sync', ?2, ?3, 'ok', 0)",
                                rusqlite::params![
                                    now,
                                    decision.source_host,
                                    format!("sync {}", decision.path)
                                ],
                            ) {
                                tracing::warn!(error = %e, "failed to record operation_log entry");
                            }
                        }

                        if !failed_uploads.is_empty() {
                            if verbose {
                                let failed_names: Vec<&str> =
                                    failed_uploads.iter().map(|(n, _)| n.as_str()).collect();
                                printer::print_host_line(
                                    "failed",
                                    "error",
                                    &failed_names.join(", "),
                                );
                            } else {
                                tracing::warn!(hosts = %failed_uploads.iter().map(|(n,_)| n.as_str()).collect::<Vec<_>>().join(", "), "upload failed");
                            }
                        }

                        summary.complete_file(
                            &decision.path,
                            &decision.synced_hosts,
                            &succeeded,
                            &failed_uploads,
                        );
                        for target in &succeeded {
                            host_file_map
                                .entry(target.clone())
                                .or_default()
                                .0
                                .push(decision.path.clone());
                        }
                        for h in &decision.synced_hosts {
                            host_file_map
                                .entry(h.clone())
                                .or_default()
                                .1
                                .push(decision.path.clone());
                        }
                    }
                    Err(e) => {
                        if verbose {
                            printer::print_host_line("failed", "error", &decision.source_host);
                        } else {
                            tracing::warn!(host = %decision.source_host, error = %e, "download from source failed");
                        }
                        let all_failed: Vec<(String, String)> =
                            std::iter::once((decision.source_host.clone(), e.to_string()))
                                .chain(
                                    decision.target_hosts.iter().map(|t| {
                                        (t.clone(), format!("source download failed: {}", e))
                                    }),
                                )
                                .collect();
                        summary.complete_file(
                            &decision.path,
                            &decision.synced_hosts,
                            &[],
                            &all_failed,
                        );
                    }
                }
            }
        }
    }

    // Handle recursive entries with per-file flow.
    for (entry, hosts_for_entry, effective_source) in &recursive_entries {
        let scoped_hosts: Vec<&HostEntry> = reachable_hosts
            .iter()
            .filter(|h| hosts_for_entry.contains(&h.name))
            .copied()
            .collect();
        if scoped_hosts.len() < 2 {
            continue;
        }

        let expanded_paths: Vec<String> = if let Some(src_name) = effective_source {
            if let Some(source_host) = scoped_hosts.iter().find(|h| h.name == *src_name) {
                match expand_directory_paths(
                    source_host,
                    &entry.paths,
                    true,
                    ctx.timeout,
                    &pool.session_pool,
                )
                .await
                {
                    Ok(expansions) => {
                        let mut paths = Vec::new();
                        for p in &entry.paths {
                            match expansions.get(p) {
                                Some(DirExpandResult::Directory(files)) => {
                                    if files.is_empty() {
                                        if verbose {
                                            println!(
                                                "  {} (empty directory on source, skipping)",
                                                p
                                            );
                                        }
                                    } else {
                                        paths.extend(files.iter().cloned());
                                    }
                                }
                                Some(DirExpandResult::File) | None => {
                                    paths.push(p.clone());
                                }
                                Some(DirExpandResult::Missing) => {
                                    paths.push(p.clone());
                                }
                            }
                        }
                        paths
                    }
                    Err(e) => {
                        tracing::warn!(
                            source = %src_name,
                            error = %e,
                            "Failed to expand directories from source, falling back to original paths"
                        );
                        entry.paths.clone()
                    }
                }
            } else {
                entry.paths.clone()
            }
        } else {
            entry.paths.clone()
        };

        for path in &expanded_paths {
            sync_path_across(
                ctx,
                &scoped_hosts,
                path,
                &label,
                dry_run,
                push_missing,
                *effective_source,
                Arc::clone(&pool.session_pool),
                &mut summary,
                !verbose,
            )
            .await?;
        }
    }

    let failed_host_names: Vec<String> = {
        let mut names = Vec::new();
        for (name, _) in pool.failed_hosts() {
            names.push(name);
        }
        for (name, _) in pool.sftp_failed_hosts() {
            names.push(name);
        }
        names
    };

    pool.shutdown().await;

    if verbose {
        summary.print();
    }

    let report = build_sync_report(
        executed_at,
        mode,
        dry_run,
        &host_names,
        &host_file_map,
        &summary,
        &failed_host_names,
        &requested_paths,
    );

    if let Some(p) = progress {
        for hr in &report.hosts {
            p.host_completed(&hr.host, hr.status, &hr.detail, hr.duration_ms.unwrap_or(0));
        }
    }

    Ok(CommandReport::Sync(report))
}

#[allow(clippy::too_many_arguments)]
async fn sync_path_across(
    ctx: &Context,
    hosts: &[&HostEntry],
    path: &str,
    group_name: &str,
    dry_run: bool,
    push_missing: bool,
    source_override: Option<&str>,
    sessions: Arc<RusshSessionPool>,
    summary: &mut SyncSummary,
    quiet: bool,
) -> Result<()> {
    let collect_result = collect_file_metadata(
        hosts,
        path,
        ctx.timeout,
        ctx.concurrency(),
        Arc::clone(&sessions),
    )
    .await?;

    if collect_result.found.is_empty() {
        if !collect_result.missing.is_empty() {
            if !quiet {
                println!("  {}: file not found on any reachable host", path);
            } else {
                tracing::debug!(path, "file not found on any reachable host");
            }
        } else if !quiet {
            println!("  {}: no data collected", path);
        } else {
            tracing::debug!(path, "no data collected");
        }
        return Ok(());
    }

    let decisions = if let Some(src) = source_override {
        let (decs, skip_info) = make_decisions_fixed_source(
            &collect_result.found,
            path,
            push_missing,
            &collect_result.missing,
            src,
        )?;
        if let Some((source, skipped_path)) = skip_info {
            if !quiet {
                printer::print_host_line(
                    "skip",
                    &source,
                    &format!("does not have '{}'", skipped_path),
                );
            } else {
                tracing::debug!(host = %source, path = %skipped_path, "source does not have path, skipping");
            }
            summary.add_skip_with_reason(
                &skipped_path,
                &source,
                &format!("source '{}' does not have '{}'", source, skipped_path),
            );
            return Ok(());
        }
        decs
    } else {
        make_decisions(
            &collect_result.found,
            &ctx.config.settings.conflict_strategy,
            path,
            push_missing,
            &collect_result.missing,
        )
    };

    if decisions.is_empty() {
        let hosts_list: Vec<&str> = collect_result
            .found
            .iter()
            .map(|f| f.host.as_str())
            .collect();
        if !quiet {
            println!("  {} (all in sync)", path);
            printer::print_host_line("passed", "ok", &hosts_list.join(", "));
        } else {
            tracing::debug!(path, hosts = %hosts_list.join(", "), "all in sync");
        }
        summary.file_in_sync(&hosts_list);
        return Ok(());
    }

    for decision in &decisions {
        let mut all_targets: Vec<&str> = decision.synced_hosts.iter().map(|s| s.as_str()).collect();
        all_targets.extend(decision.target_hosts.iter().map(|s| s.as_str()));
        if !quiet {
            println!(
                "  {} → source: {} → targets: [{}] ({})",
                decision.path,
                decision.source_host,
                all_targets.join(", "),
                decision.reason
            );
        } else {
            tracing::debug!(
                path = %decision.path, source = %decision.source_host,
                targets = %all_targets.join(", "), reason = %decision.reason
            );
        }

        if !decision.synced_hosts.is_empty() {
            if !quiet {
                printer::print_host_line("passed", "ok", &decision.synced_hosts.join(", "));
            } else {
                tracing::debug!(hosts = %decision.synced_hosts.join(", "), "already in sync");
            }
        }

        if dry_run {
            continue;
        }

        match distribute(
            hosts,
            decision,
            ctx.timeout,
            ctx.concurrency(),
            Arc::clone(&sessions),
        )
        .await
        {
            Ok((succeeded, failed_uploads)) => {
                if !succeeded.is_empty() {
                    if !quiet {
                        printer::print_host_line("synced", "ok", &succeeded.join(", "));
                    } else {
                        tracing::debug!(hosts = %succeeded.join(", "), "synced");
                    }

                    let now = chrono::Utc::now().timestamp();
                    for target in &succeeded {
                        if let Err(e) = ctx.db.execute(
                            "INSERT INTO sync_state (sync_group, host, path, mtime, size_bytes, blake3, synced_at) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                             ON CONFLICT(sync_group, host, path) DO UPDATE SET mtime=?4, size_bytes=?5, blake3=?6, synced_at=?7",
                            rusqlite::params![
                                group_name, target, decision.path,
                                0i64, 0i64, "", now
                            ],
                        ) {
                            tracing::warn!(error = %e, "failed to record operation_log entry");
                        }
                    }

                    if let Err(e) = ctx.db.execute(
                        "INSERT INTO operation_log (timestamp, command, host, action, status, duration_ms) \
                         VALUES (?1, 'sync', ?2, ?3, 'ok', 0)",
                        rusqlite::params![now, decision.source_host, format!("sync {}", decision.path)],
                    ) {
                        tracing::warn!(error = %e, "failed to record operation_log entry");
                    }
                }

                if !failed_uploads.is_empty() {
                    let failed_names: Vec<&str> =
                        failed_uploads.iter().map(|(n, _)| n.as_str()).collect();
                    if !quiet {
                        printer::print_host_line("failed", "error", &failed_names.join(", "));
                    } else {
                        tracing::warn!(hosts = %failed_names.join(", "), "upload failed");
                    }
                }

                summary.complete_file(
                    &decision.path,
                    &decision.synced_hosts,
                    &succeeded,
                    &failed_uploads,
                );
            }
            Err(e) => {
                if !quiet {
                    printer::print_host_line("failed", "error", &decision.source_host);
                } else {
                    tracing::warn!(host = %decision.source_host, error = %e, "download from source failed");
                }
                let all_failed: Vec<(String, String)> =
                    std::iter::once((decision.source_host.clone(), e.to_string()))
                        .chain(
                            decision
                                .target_hosts
                                .iter()
                                .map(|t| (t.clone(), format!("source download failed: {}", e))),
                        )
                        .collect();
                summary.complete_file(&decision.path, &decision.synced_hosts, &[], &all_failed);
            }
        }
    }

    if dry_run && !quiet {
        println!("  [dry-run] No changes applied.");
    }

    Ok(())
}

#[cfg(test)]
mod tests;

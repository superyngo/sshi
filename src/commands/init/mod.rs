//! Initialize sshi: generate config, test connectivity, populate known_hosts.
//!
//! This is the CLI wrapper: it owns the interactive `stdin` prompts and the
//! `println!`/`printer::*` narration. The non-interactive core helpers live
//! in [`core`]: [`core::init_core`] runs the post-prompt detect-and-persist
//! phase; [`core::batch_keyscan_and_accept`], [`core::partition_*`], and
//! friends are the work units the wrapper invokes between prompts.
//!
//! Byte-stream is identical to the legacy monolithic `init::run`: prompts
//! and per-host printer lines interleave in the original order because the
//! wrapper drives each phase (keyscan prompt → keyscan work → auth prompt →
//! per-host ssh-copy-id work) inline. The audit's suggested
//! `offer_keyscan_retry` / `offer_ssh_copy_id_retry` helpers are not
//! extracted because the per-host ssh-copy-id prompt+work interleaving
//! cannot be batched without reordering the byte stream.

pub mod core;
pub mod report;

#[cfg(test)]
mod tests;

use anyhow::Result;

use std::sync::Arc;

use crate::commands::report::printer_sink_with_skip;
use crate::config::schema::HostEntry;
use crate::config::ssh_config;
use crate::host::session_pool::RusshSessionPool;
use crate::host::session_pool::SessionPool;
use crate::output::printer;
use crate::output::progress::SyncProgress;
use crate::output::summary::Summary;

use super::Context;

use core::{
    batch_keyscan_and_accept, default_ssh_key_exists, init_core, partition_auth_failures,
    partition_host_key_failures, persist_init_result, run_interactive, InitPools,
};
use report::InitPlan;

/// Prompt the user with a yes/no question on the real terminal. Returns true on
/// an affirmative "y" answer.
fn prompt_yes_no(question: &str) -> Result<bool> {
    print!("{question} [y/N]: ");
    std::io::Write::flush(&mut std::io::stdout())?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case("y"))
}

/// Thin CLI wrapper: parses `~/.ssh/config`, asks the user the interactive
/// questions in the original byte-order, drives the non-interactive
/// [`core`] helpers between prompts, and prints the post-run summary.
pub async fn run(ctx: &Context, update: bool, dry_run: bool, skip: Vec<String>) -> Result<()> {
    println!("Scanning ~/.ssh/config...");
    let ssh_hosts = ssh_config::parse_ssh_config()?;

    if ssh_hosts.is_empty() {
        println!("No hosts found in ~/.ssh/config");
        return Ok(());
    }

    let config_exists = crate::config::app::resolve_path(ctx.config_path.as_deref())?.exists();
    let effective_update = update || config_exists;

    let mut stale_host_names: Vec<String> = Vec::new();
    let mut plan = InitPlan {
        dry_run,
        update: effective_update,
        skip: skip.clone(),
        ..Default::default()
    };

    if config_exists {
        let ssh_host_names: std::collections::HashSet<&str> =
            ssh_hosts.iter().map(|h| h.name.as_str()).collect();
        stale_host_names = ctx
            .config
            .host
            .iter()
            .filter(|h| !ssh_host_names.contains(h.ssh_host.as_str()))
            .map(|h| h.ssh_host.clone())
            .collect();

        if !stale_host_names.is_empty() {
            println!(
                "\nFound {} host(s) no longer in ~/.ssh/config:",
                stale_host_names.len()
            );
            for name in &stale_host_names {
                println!("  - {}", name);
            }

            if dry_run {
                println!(
                    "[dry-run] Would remove {} stale host(s).",
                    stale_host_names.len()
                );
            } else {
                let q = format!(
                    "Remove these {} host(s) from sshi config?",
                    stale_host_names.len()
                );
                if prompt_yes_no(&q)? {
                    plan.remove_stale_hosts = true;
                    println!("Removed {} stale host(s).", stale_host_names.len());
                }
            }
        }
    }

    let all_skips: Vec<String> = ctx
        .config
        .settings
        .skipped_hosts
        .iter()
        .cloned()
        .chain(skip.iter().cloned())
        .collect();

    let mut detect_hosts: Vec<String> = Vec::new();
    let mut summary = Summary::default();

    for ssh_host in &ssh_hosts {
        if all_skips.iter().any(|s| s == &ssh_host.name) {
            printer::print_host_line(&ssh_host.name, "skip", "skipped");
            summary.add_skip();
            continue;
        }
        let already_exists = ctx.config.host.iter().any(|h| h.ssh_host == ssh_host.name);
        if already_exists && !effective_update {
            continue;
        }
        detect_hosts.push(ssh_host.name.clone());
    }

    if detect_hosts.is_empty() {
        if skip.is_empty() {
            println!("No new hosts to detect.");
        }

        if !skip.is_empty() {
            summary.print();

            if dry_run {
                println!("\n[dry-run] No changes written.");
                return Ok(());
            }

            let path = persist_init_result(ctx, &[], &plan, &stale_host_names)?;
            if let Some(p) = &path {
                println!("\nConfig saved to {}", p.display());
            }
            return Ok(());
        }

        if plan.remove_stale_hosts {
            let path = persist_init_result(ctx, &[], &plan, &stale_host_names)?;
            if let Some(p) = &path {
                println!("\nConfig saved to {}", p.display());
            }
        }
        return Ok(());
    }

    println!(
        "Found {} host(s). Detecting shell types...",
        detect_hosts.len()
    );

    let temp_entries: Vec<Arc<HostEntry>> = detect_hosts
        .iter()
        .map(|name| Arc::new(HostEntry::placeholder(name, name)))
        .collect();

    let mut progress = SyncProgress::new();
    progress.start_host_check(temp_entries.len());
    let session_pool =
        RusshSessionPool::setup(&temp_entries, ctx.timeout, ctx.concurrency(), None).await?;
    let connected = session_pool.reachable_hosts().len();
    let failed_count = temp_entries.len() - connected;
    progress.finish_host_check(connected, failed_count);

    let (host_key_failures, rest) = partition_host_key_failures(session_pool.failed_hosts());
    let (auth_failures, other_failures) = partition_auth_failures(rest);

    for (name, err) in &other_failures {
        printer::print_host_line(name, "error", err);
        summary.add_failure(name, err);
    }

    let mut retry_pool: Option<RusshSessionPool> = None;
    if !host_key_failures.is_empty() {
        if dry_run {
            for (name, _err) in &host_key_failures {
                printer::print_host_line(name, "skip", "unknown host key (dry-run, skipped)");
                summary.add_skip();
            }
        } else {
            println!(
                "\n{} host(s) have unknown SSH host keys:",
                host_key_failures.len()
            );
            for (name, _err) in &host_key_failures {
                println!("  - {}", name);
            }
            print!("Add to known_hosts and retry? [y/N]: ");
            std::io::Write::flush(&mut std::io::stdout())?;
            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;

            if answer.trim().eq_ignore_ascii_case("y") {
                plan.accept_unknown_host_keys = true;
                let accepted = batch_keyscan_and_accept(
                    &host_key_failures,
                    ctx.timeout,
                    ctx.concurrency(),
                    None,
                )
                .await;

                for (name, _err) in &host_key_failures {
                    if !accepted.contains(name) {
                        printer::print_host_line(name, "error", "keyscan failed");
                        summary.add_failure(name, "keyscan failed");
                    }
                }

                if !accepted.is_empty() {
                    let retry_entries: Vec<Arc<HostEntry>> = accepted
                        .iter()
                        .map(|name| Arc::new(HostEntry::placeholder(name, name)))
                        .collect();

                    println!("\nRetrying {} host(s)...", accepted.len());
                    progress.start_host_check(retry_entries.len());
                    let rp = RusshSessionPool::setup(
                        &retry_entries,
                        ctx.timeout,
                        ctx.concurrency(),
                        None,
                    )
                    .await?;
                    let retry_connected = rp.reachable_hosts().len();
                    let retry_failed = retry_entries.len() - retry_connected;
                    progress.finish_host_check(retry_connected, retry_failed);

                    for (name, err) in rp.failed_hosts() {
                        printer::print_host_line(&name, "error", &err);
                        summary.add_failure(&name, &err);
                    }

                    retry_pool = Some(rp);
                }
            } else {
                for (name, err) in &host_key_failures {
                    printer::print_host_line(name, "error", err);
                    summary.add_failure(name, err);
                }
            }
        }
    }

    let mut auth_retry_pool: Option<RusshSessionPool> = None;
    if !auth_failures.is_empty() {
        if dry_run {
            for (name, _err) in &auth_failures {
                printer::print_host_line(name, "skip", "key auth failed (dry-run, skipped)");
                summary.add_skip();
            }
        } else {
            println!(
                "\n{} host(s) could not authenticate with an SSH key:",
                auth_failures.len()
            );
            for (name, _err) in &auth_failures {
                println!("  - {}", name);
            }

            let have_default_key = default_ssh_key_exists();
            if !have_default_key
                && prompt_yes_no("No SSH key found. Create one (ssh-keygen -t ed25519)?")?
            {
                plan.generate_ssh_key_if_missing = true;
            }

            let mut have_key = have_default_key;
            if plan.generate_ssh_key_if_missing {
                have_key = run_interactive("ssh-keygen", &["-t", "ed25519"]);
                if !have_key {
                    println!("ssh-keygen did not produce a key; skipping ssh-copy-id.");
                }
            }

            let mut copied: Vec<String> = Vec::new();
            if have_key {
                for (name, _err) in &auth_failures {
                    let q = format!("Copy your public key to '{name}' (ssh-copy-id)?");
                    if prompt_yes_no(&q)? {
                        if run_interactive("ssh-copy-id", &[name]) {
                            printer::print_host_line(name, "ok", "public key copied");
                            copied.push(name.clone());
                        } else {
                            printer::print_host_line(name, "error", "ssh-copy-id failed");
                            summary.add_failure(name, "ssh-copy-id failed");
                        }
                    } else {
                        printer::print_host_line(name, "skip", "key copy declined");
                        summary.add_skip();
                    }
                }
            } else {
                for (name, err) in &auth_failures {
                    printer::print_host_line(name, "error", err);
                    summary.add_failure(name, err);
                }
            }

            if !copied.is_empty() {
                let retry_entries: Vec<Arc<HostEntry>> = copied
                    .iter()
                    .map(|name| Arc::new(HostEntry::placeholder(name, name)))
                    .collect();

                println!("\nRetrying {} host(s)...", copied.len());
                progress.start_host_check(retry_entries.len());
                let rp =
                    RusshSessionPool::setup(&retry_entries, ctx.timeout, ctx.concurrency(), None)
                        .await?;
                let retry_connected = rp.reachable_hosts().len();
                let retry_failed = retry_entries.len() - retry_connected;
                progress.finish_host_check(retry_connected, retry_failed);

                for (name, err) in rp.failed_hosts() {
                    printer::print_host_line(&name, "error", &err);
                    summary.add_failure(&name, &err);
                }

                auth_retry_pool = Some(rp);
            }
        }
    }

    // Shell detection + persist delegated to `init_core`. The detect phase
    // streams per-host events via the sink; the wrapper folds the typed
    // results into its local summary afterwards. `InitPools` fields are
    // `&dyn SessionPool`; the unsized coercion from `&RusshSessionPool`
    // happens at field assignment for the bare reference, and via `.map()`
    // for the `Option<&_>` fields (Rust does not auto-coerce through Option).
    let sink = printer_sink_with_skip();
    let pools = InitPools {
        session: &session_pool,
        retry: retry_pool.as_ref().map(|p| p as &dyn SessionPool),
        auth_retry: auth_retry_pool.as_ref().map(|p| p as &dyn SessionPool),
    };
    let report = init_core(ctx, &ssh_hosts, pools, &plan, Some(&sink)).await?;

    for _ in &report.detected_hosts {
        summary.add_success();
    }
    for f in &report.failed_hosts {
        summary.add_failure(&f.host, &f.detail);
    }

    session_pool.shutdown().await;
    if let Some(rp) = retry_pool {
        rp.shutdown().await;
    }
    if let Some(rp) = auth_retry_pool {
        rp.shutdown().await;
    }
    progress.clear();
    summary.print();

    if dry_run {
        println!("\n[dry-run] No changes written.");
        return Ok(());
    }

    if report.new_hosts.is_empty() && skip.is_empty() && !plan.remove_stale_hosts {
        println!("No new hosts to add.");
        return Ok(());
    }

    if let Some(path) = &report.persisted_path {
        println!("\nConfig saved to {}", path.display());
    }

    Ok(())
}

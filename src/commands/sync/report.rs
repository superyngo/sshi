use std::collections::HashMap;

use crate::output::summary::SyncSummary;

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_sync_report(
    executed_at: String,
    mode: &str,
    dry_run: bool,
    host_names: &[String],
    host_file_map: &HashMap<String, (Vec<String>, Vec<String>)>,
    summary: &SyncSummary,
    failed_hosts: &[String],
    paths: &[String],
) -> crate::commands::report::SyncReport {
    use crate::commands::report::{HostStatus, SyncHostResult, SyncReport};

    let mut host_errors: HashMap<String, Vec<String>> = HashMap::new();
    for err in &summary.errors {
        host_errors
            .entry(err.host.clone())
            .or_default()
            .push(err.message.clone());
    }

    let hosts: Vec<SyncHostResult> = host_names
        .iter()
        .map(|name| {
            let (synced_paths, skipped_paths) =
                host_file_map.get(name).cloned().unwrap_or_default();
            let (synced, skipped) = (synced_paths.len(), skipped_paths.len());
            let errors = host_errors.get(name).cloned().unwrap_or_default();
            let status = if failed_hosts.contains(name) {
                HostStatus::Unreachable
            } else if !errors.is_empty() {
                HostStatus::Error
            } else {
                HostStatus::Online
            };
            let detail = if failed_hosts.contains(name) {
                errors
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "unreachable".to_string())
            } else {
                format!("{} synced, {} skipped", synced, skipped)
            };
            SyncHostResult {
                host: name.clone(),
                status,
                duration_ms: None,
                detail,
                files_synced: synced,
                files_skipped: skipped,
                synced_paths,
                skipped_paths,
                errors,
            }
        })
        .collect();

    SyncReport {
        executed_at,
        mode: mode.to_string(),
        dry_run,
        total_files_synced: summary.files_synced,
        total_files_skipped: summary.files_skipped,
        paths: paths.to_vec(),
        targets: host_names.to_vec(),
        hosts,
    }
}

use anyhow::Result;

use crate::config::schema::ConflictStrategy;

use super::types::{FileInfo, SkipInfo, SyncDecision};

pub(crate) fn make_decisions(
    file_infos: &[FileInfo],
    strategy: &ConflictStrategy,
    path: &str,
    push_missing: bool,
    missing_hosts: &[String],
) -> Vec<SyncDecision> {
    if file_infos.is_empty() {
        return Vec::new();
    }

    let first_hash = &file_infos[0].hash;
    let all_in_sync = !first_hash.is_empty() && file_infos.iter().all(|f| f.hash == *first_hash);

    if all_in_sync && (!push_missing || missing_hosts.is_empty()) {
        return Vec::new();
    }

    match strategy {
        ConflictStrategy::Newest => {
            let source = file_infos.iter().max_by_key(|f| f.mtime).unwrap();

            let mut targets: Vec<String> = file_infos
                .iter()
                .filter(|f| f.host != source.host)
                .filter(|f| f.hash != source.hash)
                .map(|f| f.host.clone())
                .collect();

            if push_missing {
                for h in missing_hosts {
                    if !targets.contains(h) {
                        targets.push(h.clone());
                    }
                }
            }

            if targets.is_empty() {
                return Vec::new();
            }

            let conflict_targets: Vec<_> = file_infos
                .iter()
                .filter(|f| f.host != source.host && f.hash != source.hash)
                .collect();
            let reason = if conflict_targets.is_empty() && push_missing && !missing_hosts.is_empty()
            {
                format!(
                    "in sync on reachable hosts, pushing to {} missing",
                    missing_hosts.len()
                )
            } else {
                let mut r = format!("newest mtime: {}", source.mtime);
                if push_missing && !missing_hosts.is_empty() {
                    r.push_str(&format!(", +{} missing", missing_hosts.len()));
                }
                r
            };

            let synced_hosts: Vec<String> = file_infos
                .iter()
                .filter(|f| f.host != source.host && f.hash == source.hash)
                .map(|f| f.host.clone())
                .collect();

            vec![SyncDecision {
                path: path.to_string(),
                source_host: source.host.clone(),
                target_hosts: targets,
                synced_hosts,
                reason,
            }]
        }
        ConflictStrategy::Skip => {
            let hashes: std::collections::HashSet<_> = file_infos.iter().map(|f| &f.hash).collect();
            if hashes.len() > 1 {
                tracing::warn!(
                    path = %path,
                    "Conflict detected, skipping (strategy: skip)"
                );
                return Vec::new();
            }

            if push_missing && !missing_hosts.is_empty() && all_in_sync {
                let source = &file_infos[0];
                let synced_hosts: Vec<String> =
                    file_infos.iter().skip(1).map(|f| f.host.clone()).collect();
                return vec![SyncDecision {
                    path: path.to_string(),
                    source_host: source.host.clone(),
                    target_hosts: missing_hosts.to_vec(),
                    synced_hosts,
                    reason: format!("push to {} missing host(s)", missing_hosts.len()),
                }];
            }

            Vec::new()
        }
    }
}

pub(crate) fn make_decisions_fixed_source(
    file_infos: &[FileInfo],
    path: &str,
    push_missing: bool,
    missing_hosts: &[String],
    source_name: &str,
) -> Result<(Vec<SyncDecision>, SkipInfo)> {
    let source = match file_infos.iter().find(|f| f.host == source_name) {
        Some(s) => s,
        None => {
            return Ok((
                Vec::new(),
                Some((source_name.to_string(), path.to_string())),
            ));
        }
    };

    let mut targets: Vec<String> = file_infos
        .iter()
        .filter(|f| f.host != source.host)
        .filter(|f| f.hash != source.hash)
        .map(|f| f.host.clone())
        .collect();

    if push_missing {
        for h in missing_hosts {
            if !targets.contains(h) {
                targets.push(h.clone());
            }
        }
    }

    if targets.is_empty() {
        return Ok((Vec::new(), None));
    }

    let synced_hosts: Vec<String> = file_infos
        .iter()
        .filter(|f| f.host != source.host && f.hash == source.hash)
        .map(|f| f.host.clone())
        .collect();

    let mut reason = format!("fixed source: {}", source_name);
    if push_missing && !missing_hosts.is_empty() {
        reason.push_str(&format!(", +{} missing", missing_hosts.len()));
    }

    Ok((
        vec![SyncDecision {
            path: path.to_string(),
            source_host: source.host.clone(),
            target_hosts: targets,
            synced_hosts,
            reason,
        }],
        None,
    ))
}

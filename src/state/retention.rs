//! Time-based retention cleanup for snapshots and operation logs.

use anyhow::Result;

use crate::state::db::DbHandle;

/// Delete check_snapshots and operation_log entries older than retention_days.
/// If retention_days is 0, no cleanup is performed (keep forever).
pub async fn cleanup(db: &DbHandle, retention_days: u64) -> Result<()> {
    if retention_days == 0 {
        return Ok(());
    }

    let cutoff_secs = retention_days * 86400;

    db.execute(
        "DELETE FROM check_snapshots WHERE collected_at < (strftime('%s', 'now') - ?1)",
        vec![crate::state::db::boxed_param(cutoff_secs)],
    )
    .await?;

    db.execute(
        "DELETE FROM operation_log WHERE timestamp < (strftime('%s', 'now') - ?1)",
        vec![crate::state::db::boxed_param(cutoff_secs)],
    )
    .await?;

    Ok(())
}

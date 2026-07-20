//! SSH transport: session pooling, authentication, shell detection, and SFTP.

pub mod auth;
pub mod concurrency;
pub mod pool;
pub mod session_pool;
pub mod sftp;
pub mod shell;

/// Test-only `SessionPool` impl returning canned `exec` / `upload` /
/// `download` responses. Used by integration tests in
/// `commands::init::tests` and `commands::sync::integration_tests` to drive
/// `init_core` and the sync phase helpers end-to-end without a live SSH
/// server (audit §2.8 HIGH ×2).
#[cfg(test)]
pub mod session_pool_mock;

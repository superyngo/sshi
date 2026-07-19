use std::collections::HashMap;

use crate::config::schema::{ConflictStrategy, ShellType};

use super::collect::{
    build_batch_metadata_cmd, build_dir_expand_cmd, parse_batch_metadata_output,
    parse_dir_expand_output, union_dir_expansions,
};
use super::decide::{make_decisions, make_decisions_fixed_source};
use super::types::{DirExpandResult, FileInfo};

fn make_file_info(host: &str, _path: &str, hash: &str) -> FileInfo {
    FileInfo {
        host: host.to_string(),
        mtime: 1000,
        hash: hash.to_string(),
    }
}

fn make_file_info_mtime(host: &str, hash: &str, mtime: i64) -> FileInfo {
    FileInfo {
        host: host.to_string(),
        mtime,
        hash: hash.to_string(),
    }
}

#[test]
fn test_fixed_source_missing_returns_empty_not_error() {
    let infos = vec![
        make_file_info("host-b", "~/.bashrc", "abc123"),
        make_file_info("host-c", "~/.bashrc", "def456"),
    ];
    let missing: Vec<String> = vec![];
    let result = make_decisions_fixed_source(&infos, "~/.bashrc", false, &missing, "host-a");
    assert!(result.is_ok(), "should not return an error");
    let (decisions, skip_info) = result.unwrap();
    assert!(decisions.is_empty(), "decisions should be empty");
    assert!(skip_info.is_some(), "skip_info should be Some");
    let (source, path) = skip_info.unwrap();
    assert_eq!(source, "host-a");
    assert_eq!(path, "~/.bashrc");
}

#[test]
fn test_fixed_source_present_works_normally() {
    let infos = vec![
        make_file_info("host-a", "~/.bashrc", "abc123"),
        make_file_info("host-b", "~/.bashrc", "def456"),
        make_file_info("host-c", "~/.bashrc", "abc123"),
    ];
    let missing: Vec<String> = vec![];
    let result = make_decisions_fixed_source(&infos, "~/.bashrc", false, &missing, "host-a");
    assert!(result.is_ok());
    let (decisions, skip_info) = result.unwrap();
    assert!(
        skip_info.is_none(),
        "skip_info should be None when source is found"
    );
    assert_eq!(decisions.len(), 1);
    let d = &decisions[0];
    assert_eq!(d.source_host, "host-a");
    assert_eq!(d.target_hosts, vec!["host-b".to_string()]);
    assert_eq!(d.synced_hosts, vec!["host-c".to_string()]);
    assert!(d.reason.contains("fixed source: host-a"));
}

#[test]
fn test_build_batch_metadata_cmd_sh() {
    let paths = vec!["~/.bashrc".to_string(), "~/.vimrc".to_string()];
    let cmd = build_batch_metadata_cmd(&paths, ShellType::Sh);
    assert!(cmd.contains("---FILE:"));
    assert!(cmd.contains("stat"));
    assert!(cmd.contains("sha256sum") || cmd.contains("shasum"));
    assert!(cmd.contains(".bashrc"));
    assert!(cmd.contains(".vimrc"));
}

#[test]
fn test_build_batch_metadata_cmd_powershell() {
    let paths = vec!["~/.bashrc".to_string()];
    let cmd = build_batch_metadata_cmd(&paths, ShellType::PowerShell);
    assert!(cmd.contains("---FILE:"));
    assert!(cmd.contains("Get-Item"));
    assert!(cmd.contains("Get-FileHash"));
}

#[test]
fn test_parse_batch_metadata_output() {
    let output = "---FILE:$HOME/.bashrc\n1700000000 1234\nabc123def456  /home/user/.bashrc\n---FILE:$HOME/.vimrc\nMISSING\n";
    let paths = vec!["~/.bashrc".to_string(), "~/.vimrc".to_string()];
    let result = parse_batch_metadata_output(output, &paths, "host-a");
    assert_eq!(result.len(), 2);
    let bashrc = &result["~/.bashrc"];
    assert!(bashrc.found.is_some());
    let fi = bashrc.found.as_ref().unwrap();
    assert_eq!(fi.mtime, 1700000000);
    assert_eq!(fi.hash, "abc123def456");
    let vimrc = &result["~/.vimrc"];
    assert!(vimrc.found.is_none());
    assert!(vimrc.is_missing);
}

#[test]
fn test_parse_batch_metadata_nohash() {
    let output = "---FILE:$HOME/.bashrc\n1700000000 1234\nNOHASH\n";
    let paths = vec!["~/.bashrc".to_string()];
    let result = parse_batch_metadata_output(output, &paths, "host-a");
    let bashrc = &result["~/.bashrc"];
    assert!(bashrc.found.is_some());
    let fi = bashrc.found.as_ref().unwrap();
    assert!(
        fi.hash.is_empty(),
        "NOHASH sentinel should produce empty hash"
    );
}

#[test]
fn test_build_dir_expand_cmd_sh_shallow() {
    let paths = vec!["~/mydir".to_string(), "~/single.conf".to_string()];
    let cmd = build_dir_expand_cmd(&paths, false, ShellType::Sh);
    assert!(cmd.contains("---PATH:"));
    assert!(cmd.contains("[ -d"));
    assert!(cmd.contains("-maxdepth 1"));
    assert!(cmd.contains("find"));
    assert!(
        cmd.contains("find -L"),
        "find must use -L to follow symlinks"
    );
    assert!(cmd.contains("mydir"));
    assert!(cmd.contains("single.conf"));
}

#[test]
fn test_build_dir_expand_cmd_sh_recursive() {
    let paths = vec!["~/mydir".to_string()];
    let cmd = build_dir_expand_cmd(&paths, true, ShellType::Sh);
    assert!(cmd.contains("---PATH:"));
    assert!(cmd.contains("find"));
    assert!(
        cmd.contains("find -L"),
        "find must use -L to follow symlinks"
    );
    assert!(
        !cmd.contains("-maxdepth"),
        "recursive should not have maxdepth"
    );
}

#[test]
fn test_build_dir_expand_cmd_powershell() {
    let paths = vec!["~/mydir".to_string()];
    let cmd = build_dir_expand_cmd(&paths, false, ShellType::PowerShell);
    assert!(cmd.contains("---PATH:"));
    assert!(cmd.contains("Test-Path"));
    assert!(cmd.contains("Get-ChildItem"));
    assert!(
        !cmd.contains("-Recurse"),
        "shallow should not have -Recurse"
    );

    let cmd_recursive = build_dir_expand_cmd(&paths, true, ShellType::PowerShell);
    assert!(cmd_recursive.contains("-Recurse"));
}

#[test]
fn test_parse_dir_expand_output_mixed() {
    let output = "---PATH:~/mydir\nDIR\n~/mydir/file1.txt\n~/mydir/file2.txt\n---PATH:~/single.conf\nFILE\n---PATH:~/gone\nMISSING\n";
    let paths = vec![
        "~/mydir".to_string(),
        "~/single.conf".to_string(),
        "~/gone".to_string(),
    ];
    let result = parse_dir_expand_output(output, &paths);
    assert_eq!(result.len(), 3);

    match &result["~/mydir"] {
        DirExpandResult::Directory(files) => {
            assert_eq!(files.len(), 2);
            assert_eq!(files[0], "~/mydir/file1.txt");
            assert_eq!(files[1], "~/mydir/file2.txt");
        }
        other => panic!("Expected Directory, got {:?}", other),
    }

    assert_eq!(result["~/single.conf"], DirExpandResult::File);
    assert_eq!(result["~/gone"], DirExpandResult::Missing);
}

#[test]
fn test_parse_dir_expand_output_empty_dir() {
    let output = "---PATH:~/emptydir\nDIR\n";
    let paths = vec!["~/emptydir".to_string()];
    let result = parse_dir_expand_output(output, &paths);

    match &result["~/emptydir"] {
        DirExpandResult::Directory(files) => {
            assert!(files.is_empty(), "empty directory should have no files");
        }
        other => panic!("Expected Directory, got {:?}", other),
    }
}

#[test]
fn test_parse_dir_expand_output_nested() {
    let output = "---PATH:~/project\nDIR\n~/project/src/main.rs\n~/project/src/lib.rs\n~/project/Cargo.toml\n";
    let paths = vec!["~/project".to_string()];
    let result = parse_dir_expand_output(output, &paths);

    match &result["~/project"] {
        DirExpandResult::Directory(files) => {
            assert_eq!(files.len(), 3);
            assert!(files.contains(&"~/project/src/main.rs".to_string()));
            assert!(files.contains(&"~/project/src/lib.rs".to_string()));
            assert!(files.contains(&"~/project/Cargo.toml".to_string()));
        }
        other => panic!("Expected Directory, got {:?}", other),
    }
}

#[test]
fn test_union_dir_expansions_merges_files() {
    let host_a = HashMap::from([
        (
            "~/bin".to_string(),
            DirExpandResult::Directory(vec!["~/bin/f1".to_string(), "~/bin/f2".to_string()]),
        ),
        ("~/.vimrc".to_string(), DirExpandResult::File),
    ]);
    let host_b = HashMap::from([
        (
            "~/bin".to_string(),
            DirExpandResult::Directory(vec!["~/bin/f2".to_string(), "~/bin/f3".to_string()]),
        ),
        ("~/.vimrc".to_string(), DirExpandResult::File),
    ]);
    let result = union_dir_expansions(vec![host_a, host_b]);
    assert_eq!(result.len(), 1, "only ~/bin should be in result");
    let files = result.get("~/bin").expect("~/bin must be present");
    assert_eq!(files.len(), 3, "union must deduplicate f2");
    assert!(files.contains(&"~/bin/f1".to_string()));
    assert!(files.contains(&"~/bin/f2".to_string()));
    assert!(files.contains(&"~/bin/f3".to_string()));
}

#[test]
fn test_union_dir_expansions_empty_on_one_host() {
    let host_a = HashMap::from([("~/bin".to_string(), DirExpandResult::Directory(vec![]))]);
    let host_b = HashMap::from([(
        "~/bin".to_string(),
        DirExpandResult::Directory(vec!["~/bin/tool".to_string()]),
    )]);
    let result = union_dir_expansions(vec![host_a, host_b]);
    let files = result.get("~/bin").expect("~/bin must be present");
    assert_eq!(files.len(), 1);
    assert!(files.contains(&"~/bin/tool".to_string()));
}

/// All hosts have identical file content → no sync needed, empty decisions
#[test]
fn test_all_identical_no_sync_needed() {
    let infos = vec![
        make_file_info("host-a", "~/.bashrc", "hash1"),
        make_file_info("host-b", "~/.bashrc", "hash1"),
        make_file_info("host-c", "~/.bashrc", "hash1"),
    ];
    let missing: Vec<String> = vec![];
    let decisions = make_decisions(
        &infos,
        &ConflictStrategy::Newest,
        "~/.bashrc",
        false,
        &missing,
    );
    assert!(
        decisions.is_empty(),
        "identical hashes should produce no decisions"
    );
}

/// Host with the highest mtime is selected as source under Newest strategy
#[test]
fn test_newest_mtime_wins_as_source() {
    let infos = vec![
        make_file_info_mtime("host-a", "aaa", 1000),
        make_file_info_mtime("host-b", "bbb", 3000),
        make_file_info_mtime("host-c", "ccc", 2000),
    ];
    let missing: Vec<String> = vec![];
    let decisions = make_decisions(
        &infos,
        &ConflictStrategy::Newest,
        "~/.bashrc",
        false,
        &missing,
    );
    assert_eq!(decisions.len(), 1);
    let d = &decisions[0];
    assert_eq!(d.source_host, "host-b", "host-b has the newest mtime");
    assert_eq!(d.target_hosts, vec!["host-a", "host-c"]);
    assert!(d.synced_hosts.is_empty());
    assert!(d.reason.contains("3000"));
}

/// When all mtimes are equal, max_by_key still picks one deterministically
#[test]
fn test_equal_mtimes_picks_a_source() {
    let infos = vec![
        make_file_info_mtime("host-a", "hash_x", 5000),
        make_file_info_mtime("host-b", "hash_y", 5000),
        make_file_info_mtime("host-c", "hash_y", 5000),
    ];
    let missing: Vec<String> = vec![];
    let decisions = make_decisions(
        &infos,
        &ConflictStrategy::Newest,
        "~/.bashrc",
        false,
        &missing,
    );
    assert_eq!(decisions.len(), 1);
    let d = &decisions[0];
    assert!(
        d.source_host == "host-a" || d.source_host == "host-b" || d.source_host == "host-c",
        "one of the tied hosts should be picked as source"
    );
    assert_eq!(
        d.synced_hosts.len(),
        1,
        "only the host matching source hash (but not source) is synced"
    );
    let non_source = [
        String::from("host-a"),
        String::from("host-b"),
        String::from("host-c"),
    ]
    .into_iter()
    .filter(|h| *h != d.source_host)
    .collect::<Vec<_>>();
    assert!(
        d.target_hosts.len() + d.synced_hosts.len() == 2,
        "targets + synced should cover the two non-source hosts"
    );
    let mut all_non_source = d.target_hosts.clone();
    all_non_source.extend(d.synced_hosts.iter().cloned());
    all_non_source.sort();
    assert_eq!(all_non_source, non_source);
}

/// Fixed source override selects the specified host regardless of mtime
#[test]
fn test_fixed_source_overrides_mtime() {
    let infos = vec![
        make_file_info_mtime("host-a", "aaa", 3000),
        make_file_info_mtime("host-b", "bbb", 1000),
        make_file_info_mtime("host-c", "ccc", 2000),
    ];
    let missing: Vec<String> = vec![];
    let result = make_decisions_fixed_source(&infos, "~/.bashrc", false, &missing, "host-b");
    let (decisions, skip_info) = result.unwrap();
    assert!(skip_info.is_none());
    assert_eq!(decisions.len(), 1);
    let d = &decisions[0];
    assert_eq!(
        d.source_host, "host-b",
        "fixed source should override mtime ordering"
    );
    assert_eq!(d.target_hosts, vec!["host-a", "host-c"]);
    assert!(d.synced_hosts.is_empty());
}

/// When push_missing is true, hosts missing the file are added as targets
#[test]
fn test_missing_hosts_become_targets() {
    let infos = vec![
        make_file_info("host-a", "~/.bashrc", "hash1"),
        make_file_info("host-b", "~/.bashrc", "hash2"),
    ];
    let missing = vec!["host-c".to_string(), "host-d".to_string()];
    let decisions = make_decisions(
        &infos,
        &ConflictStrategy::Newest,
        "~/.bashrc",
        true,
        &missing,
    );
    assert_eq!(decisions.len(), 1);
    let d = &decisions[0];
    assert_eq!(d.target_hosts, vec!["host-a", "host-c", "host-d"]);
    assert!(d.reason.contains("+2 missing"));
}

/// When push_missing is false, missing hosts are not included in targets
#[test]
fn test_missing_hosts_not_pushed_when_flag_off() {
    let infos = vec![
        make_file_info("host-a", "~/.bashrc", "hash1"),
        make_file_info("host-b", "~/.bashrc", "hash2"),
    ];
    let missing = vec!["host-c".to_string()];
    let decisions = make_decisions(
        &infos,
        &ConflictStrategy::Newest,
        "~/.bashrc",
        false,
        &missing,
    );
    assert_eq!(decisions.len(), 1);
    let d = &decisions[0];
    assert_eq!(d.target_hosts, vec!["host-a".to_string()]);
    assert!(!d.target_hosts.contains(&"host-c".to_string()));
    assert!(!d.reason.contains("missing"));
}

/// Empty file_infos slice produces no decisions
#[test]
fn test_empty_file_list_no_decisions() {
    let infos: Vec<FileInfo> = vec![];
    let missing: Vec<String> = vec![];
    let decisions = make_decisions(
        &infos,
        &ConflictStrategy::Newest,
        "~/.bashrc",
        false,
        &missing,
    );
    assert!(decisions.is_empty());
}

/// Skip strategy returns empty decisions when hashes diverge
#[test]
fn test_skip_strategy_conflict_returns_empty() {
    let infos = vec![
        make_file_info("host-a", "~/.bashrc", "hash1"),
        make_file_info("host-b", "~/.bashrc", "hash2"),
    ];
    let missing: Vec<String> = vec![];
    let decisions = make_decisions(
        &infos,
        &ConflictStrategy::Skip,
        "~/.bashrc",
        false,
        &missing,
    );
    assert!(
        decisions.is_empty(),
        "Skip strategy should return empty on hash conflict"
    );
}

/// Skip strategy with push_missing and in-sync hosts targets only missing hosts
#[test]
fn test_skip_strategy_sync_push_missing() {
    let infos = vec![
        make_file_info("host-a", "~/.bashrc", "hash1"),
        make_file_info("host-b", "~/.bashrc", "hash1"),
    ];
    let missing = vec!["host-c".to_string()];
    let decisions = make_decisions(&infos, &ConflictStrategy::Skip, "~/.bashrc", true, &missing);
    assert_eq!(decisions.len(), 1);
    let d = &decisions[0];
    assert_eq!(d.source_host, "host-a");
    assert_eq!(d.target_hosts, vec!["host-c".to_string()]);
    assert_eq!(d.synced_hosts, vec!["host-b".to_string()]);
    assert!(d.reason.contains("1 missing"));
}

/// When file is missing on all reachable hosts, no decisions are produced
#[test]
fn test_all_hosts_missing_skipped() {
    let infos: Vec<FileInfo> = vec![];
    let missing = vec!["host-a".to_string(), "host-b".to_string()];
    let decisions = make_decisions(
        &infos,
        &ConflictStrategy::Newest,
        "~/.bashrc",
        true,
        &missing,
    );
    assert!(
        decisions.is_empty(),
        "empty file_infos should produce no decisions even with missing hosts"
    );
}

#[tokio::test]
async fn decide_batch_empty_paths_returns_empty_without_io() {
    use std::sync::Arc;

    use crate::commands::sync::decide_batch;
    use crate::commands::Context;
    use crate::host::session_pool::RusshSessionPool;

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::state::db::migrate_for_test(&conn);
    let ctx = Context {
        config: std::sync::Arc::new(crate::config::schema::AppConfig::default()),
        config_path: None,
        db: crate::state::db::DbHandle::new(conn),
        timeout: 5,
        mode: crate::commands::TargetMode::All,
        serial: false,
        skip: vec![],
        verbose: false,
        auth_sender: None,
    };
    let sessions = Arc::new(
        RusshSessionPool::setup(&[], ctx.timeout, ctx.concurrency(), None)
            .await
            .unwrap(),
    );
    let reachable_hosts: Vec<std::sync::Arc<crate::config::schema::HostEntry>> = Vec::new();
    let path_source_map: HashMap<String, Option<&str>> = HashMap::new();
    let mut summary = crate::output::summary::SyncSummary::default();

    let decisions = decide_batch(
        &ctx,
        &reachable_hosts,
        &[],
        &path_source_map,
        &None,
        &sessions,
        None,
        true,
        false,
        &mut summary,
    )
    .await
    .unwrap();

    assert!(decisions.is_empty());
    assert_eq!(summary.files_synced, 0);
    assert_eq!(summary.files_skipped, 0);
}

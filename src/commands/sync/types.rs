use std::collections::{HashMap, HashSet};

use crate::config::schema::SyncEntry;

pub(crate) type RecursiveEntry<'a> = (&'a SyncEntry, HashSet<String>, Option<&'a str>);

pub(crate) type HostPathMap = HashMap<String, HashSet<String>>;

pub(crate) type PathSourceMap<'a> = HashMap<String, Option<&'a str>>;

#[derive(Debug, Clone)]
pub(crate) struct FileInfo {
    pub(crate) host: String,
    pub(crate) mtime: i64,
    pub(crate) hash: String,
}

pub(crate) struct CollectResult {
    pub(crate) found: Vec<FileInfo>,
    pub(crate) missing: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct SyncDecision {
    pub(crate) path: String,
    pub(crate) source_host: String,
    pub(crate) target_hosts: Vec<String>,
    pub(crate) synced_hosts: Vec<String>,
    pub(crate) reason: String,
}

pub(crate) type SkipInfo = Option<(String, String)>;

pub(crate) enum SyncOutputStyle {
    Verbose,
    Quiet,
}

impl SyncOutputStyle {
    pub(crate) fn is_verbose(&self) -> bool {
        matches!(self, SyncOutputStyle::Verbose)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum DirExpandResult {
    File,
    Directory(Vec<String>),
    Missing,
}

#[derive(Debug, Clone)]
pub(crate) struct SingleFileResult {
    pub(crate) found: Option<FileInfo>,
    pub(crate) is_missing: bool,
}

pub(crate) struct BatchCollectResult {
    pub(crate) per_file: HashMap<String, CollectResult>,
}

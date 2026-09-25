//! Parser for ~/.ssh/config that resolves host aliases and connection parameters.

use anyhow::{Context, Result};

/// A parsed SSH host entry from ~/.ssh/config.
#[derive(Debug, Clone, Default)]
pub struct SshHostEntry {
    pub name: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    /// Every `IdentityFile` in the block, in file order (OpenSSH accumulates them).
    pub identity_files: Vec<String>,
    pub proxy_jump: Option<String>,
    pub identities_only: Option<bool>,
}

/// Fully resolved SSH connection parameters for a host alias.
#[derive(Debug, Clone)]
pub struct ResolvedHostConfig {
    /// The alias as stored in sshi config (used for display/lookup)
    pub alias: String,
    /// Actual DNS name or IP to connect to
    pub hostname: String,
    pub port: u16,
    pub user: String,
    /// Ordered list of identity files to try: the host block's, then
    /// `Host *`'s, or the OpenSSH default keys that exist when none is listed.
    pub identity_files: Vec<std::path::PathBuf>,
    /// First ProxyJump hop alias (None = direct connection)
    pub proxy_jump: Option<String>,
    /// Whether IdentitiesOnly is set (skip password fallback)
    pub identities_only: bool,
}

#[derive(Debug, Clone, Default)]
struct Block {
    patterns: Vec<String>,
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
    identity_files: Vec<String>,
    proxy_jump: Option<String>,
    identities_only: Option<bool>,
}

/// A parsed representation of ~/.ssh/config, supporting host-specific blocks,
/// wildcard (`Host *`) inheritance, and first-wins evaluation.
#[derive(Debug, Clone, Default)]
pub struct ParsedSshConfig {
    /// Specific (non-wildcard) host entries.
    pub hosts: Vec<SshHostEntry>,
    blocks: Vec<Block>,
}

impl ParsedSshConfig {
    /// Resolve `alias` to its full connection parameters, applying OpenSSH
    /// semantics: first obtained value wins across all matching blocks in
    /// file order, and identity files accumulate.
    pub fn query(&self, alias: &str) -> ResolvedHostConfig {
        let mut hostname: Option<String> = None;
        let mut user: Option<String> = None;
        let mut port: Option<u16> = None;
        let mut identity_files: Vec<std::path::PathBuf> = Vec::new();
        let mut proxy_jump: Option<String> = None;
        let mut identities_only: Option<bool> = None;

        for block in &self.blocks {
            if block_matches_host(&block.patterns, alias) {
                if hostname.is_none() {
                    hostname = block.hostname.clone();
                }
                if user.is_none() {
                    user = block.user.clone();
                }
                if port.is_none() {
                    port = block.port;
                }
                for f in &block.identity_files {
                    let p = crate::util::expand_tilde(std::path::Path::new(f));
                    if !identity_files.contains(&p) {
                        identity_files.push(p);
                    }
                }
                if proxy_jump.is_none() {
                    proxy_jump = block.proxy_jump.clone();
                }
                if identities_only.is_none() {
                    identities_only = block.identities_only;
                }
            }
        }

        let hostname = hostname.unwrap_or_else(|| alias.to_string());
        let user = user.unwrap_or_else(whoami::username);
        let port = port.unwrap_or(22);
        if identity_files.is_empty() {
            if let Some(home) = dirs::home_dir() {
                identity_files = default_identity_files(&home);
            }
        }
        let identities_only = identities_only.unwrap_or(false);

        ResolvedHostConfig {
            alias: alias.to_string(),
            hostname,
            port,
            user,
            identity_files,
            proxy_jump,
            identities_only,
        }
    }
}

fn block_matches_host(patterns: &[String], host: &str) -> bool {
    let mut matched = false;
    for pattern in patterns {
        if let Some(negated) = pattern.strip_prefix('!') {
            if host_matches_pattern(negated, host) {
                return false;
            }
        } else if host_matches_pattern(pattern, host) {
            matched = true;
        }
    }
    matched
}

fn host_matches_pattern(pattern: &str, host: &str) -> bool {
    let pat = pattern.to_ascii_lowercase();
    let target = host.to_ascii_lowercase();
    wildcard_match(pat.as_bytes(), target.as_bytes())
}

fn wildcard_match(pat: &[u8], text: &[u8]) -> bool {
    match (pat.first(), text.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            wildcard_match(&pat[1..], text) || (!text.is_empty() && wildcard_match(pat, &text[1..]))
        }
        (Some(b'?'), Some(_)) => wildcard_match(&pat[1..], &text[1..]),
        (Some(p), Some(t)) if p == t => wildcard_match(&pat[1..], &text[1..]),
        _ => false,
    }
}

/// OpenSSH's default identity files that exist under `home/.ssh`, used only
/// when no `IdentityFile` is configured.
pub fn default_identity_files(home: &std::path::Path) -> Vec<std::path::PathBuf> {
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|n| home.join(".ssh").join(n))
        .filter(|p| p.is_file())
        .collect()
}

/// Parse ~/.ssh/config and return a list of named host entries.
/// Skips wildcard patterns (`*`, `?`).
pub fn parse_ssh_config() -> Result<Vec<SshHostEntry>> {
    let home = dirs::home_dir().context("Cannot determine home directory")?;
    let config_path = home.join(".ssh").join("config");

    if !config_path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;

    let parsed = parse_ssh_config_content_with_dir(&content, config_path.parent(), 0)?;
    Ok(parsed.hosts)
}

/// Parse raw SSH config text into a `ParsedSshConfig`.
#[cfg(test)]
pub(crate) fn parse_ssh_config_content(content: &str) -> Result<ParsedSshConfig> {
    let base = dirs::home_dir().map(|h| h.join(".ssh"));
    parse_ssh_config_content_with_dir(content, base.as_deref(), 0)
}

/// `Include` nesting limit (OpenSSH uses 16; a small cap is enough here).
const MAX_INCLUDE_DEPTH: usize = 5;

/// Blocks from the file(s) an `Include` value names. Relative paths resolve
/// against `base_dir` (`~/.ssh`), as OpenSSH does for user configs; a glob in
/// the file name is expanded in sorted order. Problems are warned, not fatal.
fn include_blocks(value: &str, base_dir: Option<&std::path::Path>, depth: usize) -> Vec<Block> {
    if depth >= MAX_INCLUDE_DEPTH {
        tracing::warn!("ssh_config Include nested too deeply at '{value}'; ignored");
        return Vec::new();
    }
    let path = crate::util::expand_tilde(std::path::Path::new(value.trim_matches('"')));
    let resolved = match base_dir {
        Some(base) if path.is_relative() => base.join(path),
        _ => path,
    };
    let files = include_files(&resolved);
    if files.is_empty() {
        tracing::warn!(
            "ssh_config Include '{}' matched no file",
            resolved.display()
        );
    }
    let mut blocks = Vec::new();
    for file in files {
        match std::fs::read_to_string(&file) {
            Ok(content) => match parse_ssh_config_content_with_dir(&content, base_dir, depth + 1) {
                Ok(sub) => blocks.extend(sub.blocks),
                Err(e) => tracing::warn!("Failed to parse included {}: {e}", file.display()),
            },
            Err(e) => tracing::warn!("Failed to read included {}: {e}", file.display()),
        }
    }
    blocks
}

/// Files matched by an `Include` path: the path itself if it exists, or the
/// sorted, non-hidden entries of its directory matching a `*`/`?` file name.
fn include_files(resolved: &std::path::Path) -> Vec<std::path::PathBuf> {
    let name = resolved
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !name.contains(['*', '?']) {
        return if resolved.is_file() {
            vec![resolved.to_path_buf()]
        } else {
            Vec::new()
        };
    }
    let Some(dir) = resolved.parent() else {
        return Vec::new();
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!("Failed to read Include directory {}: {e}", dir.display());
            return Vec::new();
        }
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter(|entry| {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            !file_name.starts_with('.') && wildcard_match(name.as_bytes(), file_name.as_bytes())
        })
        .map(|entry| entry.path())
        .collect();
    files.sort();
    files
}

fn parse_ssh_config_content_with_dir(
    content: &str,
    base_dir: Option<&std::path::Path>,
    depth: usize,
) -> Result<ParsedSshConfig> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut current: Option<Block> = None;

    for line in content.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() || trimmed_line.starts_with('#') {
            continue;
        }

        let (key, value) = if let Some(eq_pos) = trimmed_line.find('=') {
            (
                trimmed_line[..eq_pos].trim(),
                trimmed_line[eq_pos + 1..].trim(),
            )
        } else if let Some(space_pos) = trimmed_line.find(char::is_whitespace) {
            (
                trimmed_line[..space_pos].trim(),
                trimmed_line[space_pos..].trim(),
            )
        } else {
            continue;
        };

        match key.to_ascii_lowercase().as_str() {
            "host" => {
                if let Some(b) = current.take() {
                    blocks.push(b);
                }
                current = Some(Block {
                    patterns: value.split_whitespace().map(|s| s.to_string()).collect(),
                    hostname: None,
                    user: None,
                    port: None,
                    identity_files: Vec::new(),
                    proxy_jump: None,
                    identities_only: None,
                });
            }
            "match" => {
                if let Some(b) = current.take() {
                    blocks.push(b);
                }
                if value.trim().eq_ignore_ascii_case("all") {
                    current = Some(Block {
                        patterns: vec!["*".to_string()],
                        hostname: None,
                        user: None,
                        port: None,
                        identity_files: Vec::new(),
                        proxy_jump: None,
                        identities_only: None,
                    });
                } else {
                    tracing::warn!(
                        "Unsupported Match directive '{}'; skipping block",
                        trimmed_line
                    );
                    current = None;
                }
            }
            "include" => {
                // An Include inside a Host block does not end that block (OpenSSH):
                // later lines still apply to it, after the included blocks.
                let continuation = current.as_ref().map(|b| b.patterns.clone());
                if let Some(b) = current.take() {
                    blocks.push(b);
                }
                blocks.extend(include_blocks(value, base_dir, depth));
                current = continuation.map(|patterns| Block {
                    patterns,
                    ..Default::default()
                });
            }
            "hostname" => {
                if let Some(b) = current.as_mut() {
                    if b.hostname.is_none() {
                        b.hostname = Some(value.to_string());
                    }
                }
            }
            "user" => {
                if let Some(b) = current.as_mut() {
                    if b.user.is_none() {
                        b.user = Some(value.to_string());
                    }
                }
            }
            "port" => {
                if let Some(b) = current.as_mut() {
                    if b.port.is_none() {
                        b.port = value.parse().ok();
                    }
                }
            }
            "identityfile" => {
                if let Some(b) = current.as_mut() {
                    b.identity_files.push(value.to_string());
                }
            }
            "identitiesonly" => {
                if let Some(b) = current.as_mut() {
                    if b.identities_only.is_none() {
                        b.identities_only = Some(value.eq_ignore_ascii_case("yes"));
                    }
                }
            }
            "proxyjump" => {
                if let Some(b) = current.as_mut() {
                    if b.proxy_jump.is_none() {
                        let first_hop = value.split(',').next().unwrap_or(value).trim().to_string();
                        b.proxy_jump = Some(first_hop);
                    }
                }
            }
            _ => {}
        }
    }
    if let Some(b) = current.take() {
        blocks.push(b);
    }

    let mut hosts: Vec<SshHostEntry> = Vec::new();
    for block in &blocks {
        for name in &block.patterns {
            if is_wildcard(name) {
                continue;
            }
            if let Some(existing) = hosts.iter_mut().find(|h| h.name.eq_ignore_ascii_case(name)) {
                if existing.hostname.is_none() {
                    existing.hostname = block.hostname.clone();
                }
                if existing.user.is_none() {
                    existing.user = block.user.clone();
                }
                if existing.port.is_none() {
                    existing.port = block.port;
                }
                for f in &block.identity_files {
                    if !existing.identity_files.contains(f) {
                        existing.identity_files.push(f.clone());
                    }
                }
                if existing.proxy_jump.is_none() {
                    existing.proxy_jump = block.proxy_jump.clone();
                }
                if existing.identities_only.is_none() {
                    existing.identities_only = block.identities_only;
                }
            } else {
                hosts.push(SshHostEntry {
                    name: name.clone(),
                    hostname: block.hostname.clone(),
                    user: block.user.clone(),
                    port: block.port,
                    identity_files: block.identity_files.clone(),
                    proxy_jump: block.proxy_jump.clone(),
                    identities_only: block.identities_only,
                });
            }
        }
    }

    Ok(ParsedSshConfig { hosts, blocks })
}

fn is_wildcard(name: &str) -> bool {
    name.contains('*') || name.contains('?') || name.starts_with('!')
}

/// Load and parse `~/.ssh/config`.
/// For bulk use (multiple hosts), prefer `load_ssh_config()` once and call
/// `config.query(alias)` in a loop.
pub fn load_ssh_config() -> Result<ParsedSshConfig> {
    let home = dirs::home_dir().context("Cannot determine home directory")?;
    let config_path = home.join(".ssh").join("config");

    if !config_path.exists() {
        return Ok(ParsedSshConfig::default());
    }

    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;

    parse_ssh_config_content_with_dir(&content, config_path.parent(), 0)
}

/// Resolve a host alias using a pre-loaded `ParsedSshConfig`.
pub fn resolve_host_with_config(
    alias: &str,
    config: &ParsedSshConfig,
) -> Result<ResolvedHostConfig> {
    Ok(config.query(alias))
}

/// Resolve a host alias to its full connection parameters.
/// For bulk use, prefer `load_ssh_config()` + `resolve_host_with_config()`.
pub fn resolve_host(alias: &str) -> Result<ResolvedHostConfig> {
    let config = load_ssh_config()?;
    resolve_host_with_config(alias, &config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_files_accumulate_host_then_wildcard_deduped() {
        let content = "
Host a
    IdentityFile /k/one
    IdentityFile /k/two
    IdentitiesOnly yes
Host *
    IdentityFile /k/two
    IdentityFile /k/star
";
        let r = parse_ssh_config_content(content).unwrap().query("a");
        let want: Vec<std::path::PathBuf> = ["/k/one", "/k/two", "/k/star"]
            .iter()
            .map(std::path::PathBuf::from)
            .collect();
        assert_eq!(r.identity_files, want);
        assert!(r.identities_only);
    }

    #[test]
    fn identities_only_inherits_from_wildcard_and_defaults_off() {
        let c = parse_ssh_config_content(
            "Host a\nHost b\n  IdentitiesOnly no\nHost *\n  IdentitiesOnly yes\n",
        )
        .unwrap();
        assert!(c.query("a").identities_only);
        assert!(!c.query("b").identities_only);
        assert!(
            !parse_ssh_config_content("Host a\n")
                .unwrap()
                .query("a")
                .identities_only
        );
    }

    #[test]
    fn default_identity_files_lists_only_existing_in_openssh_order() {
        let t = tempfile::tempdir().unwrap();
        let ssh = t.path().join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        assert!(default_identity_files(t.path()).is_empty());
        std::fs::write(ssh.join("id_rsa"), "").unwrap();
        std::fs::write(ssh.join("id_ed25519"), "").unwrap();
        assert_eq!(
            default_identity_files(t.path()),
            vec![ssh.join("id_ed25519"), ssh.join("id_rsa")]
        );
    }

    #[test]
    fn test_parse_basic_config() {
        let content = r#"
Host home-linux
    HostName 192.168.1.10
    User alice
    Port 22
    IdentityFile ~/.ssh/id_rsa

Host work-windows
    HostName 10.0.0.5
    User bob

Host *
    ServerAliveInterval 60
"#;
        let config = parse_ssh_config_content(content).unwrap();
        assert_eq!(config.hosts.len(), 2);
        assert_eq!(config.hosts[0].name, "home-linux");
        assert_eq!(config.hosts[0].hostname.as_deref(), Some("192.168.1.10"));
        assert_eq!(config.hosts[0].user.as_deref(), Some("alice"));
        assert_eq!(config.hosts[0].port, Some(22));
        assert_eq!(config.hosts[1].name, "work-windows");
        assert_eq!(config.hosts[1].hostname.as_deref(), Some("10.0.0.5"));
    }

    #[test]
    fn test_skip_wildcards_in_hosts_list() {
        let content = "Host *\n    ServerAliveInterval 60\n\nHost ?\n    Foo bar\n";
        let config = parse_ssh_config_content(content).unwrap();
        assert!(config.hosts.is_empty());
    }

    #[test]
    fn test_multi_alias_parsed_as_separate_hosts() {
        let content = r#"
Host bastion.bss-qa slb225
    HostName 10.0.0.1
    User admin
    Port 2222

Host web1
    HostName 192.168.1.10
"#;
        let config = parse_ssh_config_content(content).unwrap();
        assert_eq!(config.hosts.len(), 3);
        let bss = config
            .hosts
            .iter()
            .find(|h| h.name == "bastion.bss-qa")
            .unwrap();
        let slb = config.hosts.iter().find(|h| h.name == "slb225").unwrap();
        let web = config.hosts.iter().find(|h| h.name == "web1").unwrap();
        assert_eq!(bss.hostname.as_deref(), Some("10.0.0.1"));
        assert_eq!(bss.port, Some(2222));
        assert_eq!(slb.hostname.as_deref(), Some("10.0.0.1"));
        assert_eq!(slb.port, Some(2222));
        assert_eq!(web.hostname.as_deref(), Some("192.168.1.10"));
    }

    #[test]
    fn test_proxy_jump_parsed() {
        let content = r#"
Host internal
    HostName 10.10.0.5
    ProxyJump bastion
"#;
        let config = parse_ssh_config_content(content).unwrap();
        assert_eq!(config.hosts[0].proxy_jump.as_deref(), Some("bastion"));
    }

    #[test]
    fn test_wildcard_inheritance_fills_missing_fields() {
        let content = r#"
Host specific
    HostName 10.0.0.1

Host *
    User defaultuser
    IdentityFile ~/.ssh/id_rsa
    Port 2222
"#;
        let config = parse_ssh_config_content(content).unwrap();
        let resolved = config.query("specific");
        assert_eq!(resolved.hostname, "10.0.0.1");
        // user and identity_file come from wildcard block
        assert_eq!(resolved.user, "defaultuser");
        assert_eq!(resolved.port, 2222);
        assert!(!resolved.identity_files.is_empty());
    }

    #[test]
    fn test_specific_host_overrides_wildcard() {
        let content = r#"
Host myhost
    HostName 10.0.0.5
    User specialuser
    Port 443

Host *
    User defaultuser
    Port 22
"#;
        let config = parse_ssh_config_content(content).unwrap();
        let resolved = config.query("myhost");
        // Specific block takes precedence over wildcard.
        assert_eq!(resolved.user, "specialuser");
        assert_eq!(resolved.port, 443);
    }

    #[test]
    fn test_unknown_host_falls_back_to_alias_as_hostname() {
        let content = r#"
Host known
    HostName 192.168.1.1
"#;
        let config = parse_ssh_config_content(content).unwrap();
        let resolved = config.query("unknown-host");
        assert_eq!(resolved.hostname, "unknown-host");
        assert_eq!(resolved.port, 22);
    }

    /// B42: Match directives must not bleed into preceding Host blocks;
    /// queries must be case-insensitive.
    #[test]
    fn test_match_directive_does_not_bleed_into_host() {
        let content = r#"
Host web1
    User alice
    HostName 10.0.0.1

Match host web*
    User deploy
    Port 2222
"#;
        let config = parse_ssh_config_content(content).unwrap();
        let resolved = config.query("web1");
        assert_eq!(resolved.user, "alice");
        assert_eq!(resolved.hostname, "10.0.0.1");
        assert_eq!(resolved.port, 22);

        // Case-insensitive query matches web1
        let resolved_case = config.query("Web1");
        assert_eq!(resolved_case.user, "alice");
        assert_eq!(resolved_case.hostname, "10.0.0.1");
        assert_eq!(resolved_case.port, 22);
    }

    /// B42: Duplicate Host blocks merge in file order with first-obtained-wins.
    #[test]
    fn test_duplicate_host_blocks_merge_first_wins() {
        let content = r#"
Host web1
    User alice
    Port 2201

Host web1
    User bob
    Port 2202
    HostName 192.168.1.100
"#;
        let config = parse_ssh_config_content(content).unwrap();
        let resolved = config.query("web1");
        assert_eq!(resolved.user, "alice");
        assert_eq!(resolved.port, 2201);
        assert_eq!(resolved.hostname, "192.168.1.100");
    }

    /// B42: Keywords and host patterns are case-insensitive.
    #[test]
    fn test_case_insensitive_keywords_and_hosts() {
        let content = r#"
HOST ServerA
    HOSTNAME servera.example.com
    USER deployer
    PORT 2222
    IDENTITIESONLY yes
"#;
        let config = parse_ssh_config_content(content).unwrap();
        let resolved = config.query("servera");
        assert_eq!(resolved.hostname, "servera.example.com");
        assert_eq!(resolved.user, "deployer");
        assert_eq!(resolved.port, 2222);
        assert!(resolved.identities_only);
    }

    /// B42: Match all is applied like Host *.
    #[test]
    fn test_match_all_is_applied() {
        let content = r#"
Match all
    Port 2222
    User defaultuser

Host myhost
    HostName 10.0.0.1
    User myuser
"#;
        let config = parse_ssh_config_content(content).unwrap();
        let resolved = config.query("myhost");
        assert_eq!(resolved.port, 2222);
        assert_eq!(resolved.user, "defaultuser");
        assert_eq!(resolved.hostname, "10.0.0.1");
    }

    /// B42: Include directive follows files and merges their blocks.
    #[test]
    fn test_include_directive_follows_files() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sub_conf = temp_dir.path().join("sub.conf");
        std::fs::write(
            &sub_conf,
            "Host included_host\n    User inc_user\n    Port 2205\n",
        )
        .unwrap();

        let main_content = format!(
            "Include {}\nHost local_host\n    Port 2206\n",
            sub_conf.display()
        );
        let config = parse_ssh_config_content(&main_content).unwrap();

        let inc = config.query("included_host");
        assert_eq!(inc.user, "inc_user");
        assert_eq!(inc.port, 2205);

        let loc = config.query("local_host");
        assert_eq!(loc.port, 2206);
    }

    /// B42: An Include inside a Host block does not drop subsequent directives for that Host.
    #[test]
    fn test_include_inside_host_block_continues_host() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sub_conf = temp_dir.path().join("sub.conf");
        std::fs::write(&sub_conf, "Host *\n    Port 2200\n").unwrap();

        let main_content = format!(
            "Host myhost\n    User alice\n    Include {}\n    HostName 10.1.2.3\n",
            sub_conf.display()
        );
        let config = parse_ssh_config_content(&main_content).unwrap();

        let res = config.query("myhost");
        assert_eq!(res.user, "alice");
        assert_eq!(res.hostname, "10.1.2.3");
        assert_eq!(res.port, 2200);
    }
}

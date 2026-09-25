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

/// A parsed representation of ~/.ssh/config, supporting host-specific blocks
/// and wildcard (`Host *`) inheritance.  Replaces the ssh2-config crate to
/// avoid a transitive openssl-sys C dependency (via git2 → libgit2-sys).
#[derive(Debug, Clone, Default)]
pub struct ParsedSshConfig {
    /// Specific (non-wildcard) host entries.
    hosts: Vec<SshHostEntry>,
    /// Wildcard defaults merged into every host that lacks the field.
    wildcard_defaults: SshHostEntry,
}

impl ParsedSshConfig {
    /// Resolve `alias` to its full connection parameters, applying wildcard
    /// inheritance for any field not set in the host-specific block.
    pub fn query(&self, alias: &str) -> ResolvedHostConfig {
        // Find the first matching specific host block.
        let specific = self.hosts.iter().find(|h| h.name == alias);

        let d = &self.wildcard_defaults;

        let hostname = specific
            .and_then(|h| h.hostname.clone())
            .or_else(|| d.hostname.clone())
            .unwrap_or_else(|| alias.to_string());

        let user = specific
            .and_then(|h| h.user.clone())
            .or_else(|| d.user.clone())
            .unwrap_or_else(whoami::username);

        let port = specific.and_then(|h| h.port).or(d.port).unwrap_or(22);

        let mut identity_files: Vec<std::path::PathBuf> = Vec::new();
        for f in specific
            .into_iter()
            .flat_map(|h| h.identity_files.iter())
            .chain(d.identity_files.iter())
        {
            let p = crate::util::expand_tilde(std::path::Path::new(f));
            if !identity_files.contains(&p) {
                identity_files.push(p);
            }
        }
        if identity_files.is_empty() {
            if let Some(home) = dirs::home_dir() {
                identity_files = default_identity_files(&home);
            }
        }

        let identities_only = specific
            .and_then(|h| h.identities_only)
            .or(d.identities_only)
            .unwrap_or(false);

        let proxy_jump = specific
            .and_then(|h| h.proxy_jump.clone())
            .or_else(|| d.proxy_jump.clone());

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

    let parsed = parse_ssh_config_content(&content)?;
    Ok(parsed.hosts)
}

/// Parse raw SSH config text into a `ParsedSshConfig` that supports
/// `Host *` wildcard inheritance.
fn parse_ssh_config_content(content: &str) -> Result<ParsedSshConfig> {
    // Accumulate all blocks (wildcard and specific) then post-process.
    struct Block {
        names: Vec<String>,
        hostname: Option<String>,
        user: Option<String>,
        port: Option<u16>,
        identity_files: Vec<String>,
        proxy_jump: Option<String>,
        identities_only: Option<bool>,
    }

    let mut blocks: Vec<Block> = Vec::new();
    let mut current: Option<Block> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = if let Some(eq_pos) = line.find('=') {
            (line[..eq_pos].trim(), line[eq_pos + 1..].trim())
        } else if let Some(space_pos) = line.find(char::is_whitespace) {
            (line[..space_pos].trim(), line[space_pos..].trim())
        } else {
            continue;
        };

        match key.to_lowercase().as_str() {
            "host" => {
                if let Some(b) = current.take() {
                    blocks.push(b);
                }
                current = Some(Block {
                    names: value.split_whitespace().map(|s| s.to_string()).collect(),
                    hostname: None,
                    user: None,
                    port: None,
                    identity_files: Vec::new(),
                    proxy_jump: None,
                    identities_only: None,
                });
            }
            "hostname" => {
                if let Some(b) = current.as_mut() {
                    b.hostname = Some(value.to_string());
                }
            }
            "user" => {
                if let Some(b) = current.as_mut() {
                    b.user = Some(value.to_string());
                }
            }
            "port" => {
                if let Some(b) = current.as_mut() {
                    b.port = value.parse().ok();
                }
            }
            "identityfile" => {
                if let Some(b) = current.as_mut() {
                    b.identity_files.push(value.to_string());
                }
            }
            "identitiesonly" => {
                if let Some(b) = current.as_mut() {
                    b.identities_only = Some(value.eq_ignore_ascii_case("yes"));
                }
            }
            "proxyjump" => {
                if let Some(b) = current.as_mut() {
                    // Take only the first hop; multiple hops are comma-separated.
                    let first_hop = value.split(',').next().unwrap_or(value).trim().to_string();
                    b.proxy_jump = Some(first_hop);
                }
            }
            _ => {}
        }
    }
    if let Some(b) = current.take() {
        blocks.push(b);
    }

    // Separate wildcard blocks (Host *) from specific host blocks.
    let mut config = ParsedSshConfig::default();
    for block in blocks {
        let all_wildcard = block.names.iter().all(|n| is_wildcard(n));
        if all_wildcard {
            // Merge this wildcard block into the defaults (first-wins for each field).
            let d = &mut config.wildcard_defaults;
            if d.hostname.is_none() {
                d.hostname = block.hostname;
            }
            if d.user.is_none() {
                d.user = block.user;
            }
            if d.port.is_none() {
                d.port = block.port;
            }
            d.identity_files.extend(block.identity_files);
            if d.proxy_jump.is_none() {
                d.proxy_jump = block.proxy_jump;
            }
            if d.identities_only.is_none() {
                d.identities_only = block.identities_only;
            }
        } else {
            // Expand multi-alias blocks into individual SshHostEntry values.
            for name in block.names.iter().filter(|n| !is_wildcard(n)) {
                config.hosts.push(SshHostEntry {
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

    Ok(config)
}

fn is_wildcard(name: &str) -> bool {
    name.contains('*') || name.contains('?')
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

    parse_ssh_config_content(&content)
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
}

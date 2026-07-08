//! SSH authentication chain: public key + password fallback with passphrase caching.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use russh::client::Handle;
use russh_keys::key::KeyPair;
use zeroize::Zeroize;

use super::session_pool::SshHandler;

/// A string that zeroizes its contents on drop.
#[derive(Clone, Debug)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(s: String) -> Self {
        Self(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Drop for SecretString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Per-process passphrase cache: key_path → passphrase.
/// Avoids re-prompting for the same key file within a single run.
pub type PassphraseCache = HashMap<PathBuf, SecretString>;

/// A request sent from the SSH auth layer to the TUI to prompt the user for a
/// credential (passphrase or password). The responder is a oneshot channel; the
/// TUI sends the entered value back through it.
#[derive(Debug)]
pub struct SshAuthRequest {
    pub prompt: String,
    pub responder: tokio::sync::oneshot::Sender<String>,
}

/// Channel sender used to forward `SshAuthRequest` values to the TUI.
pub type SshAuthSender = tokio::sync::mpsc::UnboundedSender<SshAuthRequest>;

/// Attempt to authenticate `handle` as `user`.
///
/// Auth chain:
/// 1. For each identity file: try without passphrase (unencrypted keys)
/// 2. For each identity file: if step 1 failed, prompt for passphrase (cached per path)
/// 3. Prompt for password if `identities_only` is false
pub async fn authenticate(
    handle: &mut Handle<SshHandler>,
    user: &str,
    identity_files: &[PathBuf],
    identities_only: bool,
    cache: &mut PassphraseCache,
) -> Result<()> {
    // Step 1: try each identity file without passphrase (handles unencrypted keys)
    for path in identity_files {
        if try_pubkey(handle, user, path, None).await? {
            return Ok(());
        }
    }

    // Step 2: retry each identity file with passphrase prompt (handles encrypted keys)
    for path in identity_files {
        let passphrase = match cache.get(path) {
            Some(pp) => pp.clone(),
            None => {
                let prompt = format!("Enter passphrase for {}: ", path.display());
                let pp =
                    rpassword::prompt_password(&prompt).context("Failed to read passphrase")?;
                let secret = SecretString::new(pp.clone());
                cache.insert(path.clone(), secret);
                SecretString::new(pp)
            }
        };
        if try_pubkey(handle, user, path, Some(passphrase.as_str())).await? {
            return Ok(());
        }
    }

    // Step 3: password fallback (only if IdentitiesOnly is not set)
    if !identities_only {
        let prompt = format!("{}@<host> password: ", user);
        let password = rpassword::prompt_password(&prompt).context("Failed to read password")?;
        let password = SecretString::new(password);
        if handle
            .authenticate_password(user, password.as_str())
            .await
            .context("Password authentication failed")?
        {
            return Ok(());
        }
    }

    anyhow::bail!("All authentication methods exhausted for user '{}'", user)
}

/// Try public-key auth with an optional passphrase. Returns true if auth succeeded.
async fn try_pubkey(
    handle: &mut Handle<SshHandler>,
    user: &str,
    key_path: &Path,
    passphrase: Option<&str>,
) -> Result<bool> {
    let key_pair: KeyPair = match passphrase {
        Some(pp) if !pp.is_empty() => match russh_keys::load_secret_key(key_path, Some(pp)) {
            Ok(kp) => kp,
            Err(_) => return Ok(false),
        },
        _ => {
            match russh_keys::load_secret_key(key_path, None) {
                Ok(kp) => kp,
                Err(_) => return Ok(false), // encrypted key; will retry with passphrase in step 2
            }
        }
    };

    let authed = handle
        .authenticate_publickey(user, Arc::new(key_pair))
        .await
        .context("Public key authentication error")?;
    Ok(authed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_secret_string_new_and_as_str() {
        let s = SecretString::new("hello".to_string());
        assert_eq!(s.as_str(), "hello");
    }

    #[test]
    fn test_secret_string_clone_shares_value() {
        let s = SecretString::new("secret".to_string());
        let c = s.clone();
        assert_eq!(c.as_str(), "secret");
    }

    #[test]
    fn test_secret_string_debug() {
        let s = SecretString::new("hidden".to_string());
        let debug_str = format!("{:?}", s);
        assert!(debug_str.contains("hidden"));
    }

    #[test]
    fn test_passphrase_cache_set_and_get() {
        let mut cache: PassphraseCache = HashMap::new();
        let key = PathBuf::from("/home/user/.ssh/id_rsa");
        let secret = SecretString::new("mypassphrase".to_string());
        cache.insert(key.clone(), secret);

        assert!(cache.contains_key(&key));
        let retrieved = cache.get(&key).unwrap();
        assert_eq!(retrieved.as_str(), "mypassphrase");
    }

    #[test]
    fn test_passphrase_cache_different_keys_independent() {
        let mut cache: PassphraseCache = HashMap::new();
        let key_a = PathBuf::from("/home/user/.ssh/id_rsa");
        let key_b = PathBuf::from("/home/user/.ssh/id_ed25519");
        cache.insert(key_a.clone(), SecretString::new("pass_a".to_string()));
        cache.insert(key_b.clone(), SecretString::new("pass_b".to_string()));

        assert_eq!(cache.get(&key_a).unwrap().as_str(), "pass_a");
        assert_eq!(cache.get(&key_b).unwrap().as_str(), "pass_b");
    }

    #[test]
    fn test_passphrase_cache_missing_key_returns_none() {
        let cache: PassphraseCache = HashMap::new();
        let key = PathBuf::from("/nonexistent/key");
        assert!(!cache.contains_key(&key));
    }

    #[test]
    fn test_passphrase_cache_empty_path_as_key() {
        let mut cache: PassphraseCache = HashMap::new();
        let empty_key = PathBuf::new();
        cache.insert(empty_key.clone(), SecretString::new("val".to_string()));
        assert_eq!(cache.get(&empty_key).unwrap().as_str(), "val");
    }
}

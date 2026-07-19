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

/// Attempt to authenticate `handle` as `user` against `host_name`.
///
/// Auth chain:
/// 1. For each identity file: try without passphrase (unencrypted keys)
/// 2. For each identity file: if step 1 failed, prompt for passphrase (cached per path)
/// 3. Prompt for password if `identities_only` is false
///
/// When `auth_sender` is `Some`, credential prompts are routed through the
/// TUI auth bridge (`SshAuthRequest` over the supplied mpsc, reply via the
/// embedded oneshot). When `None`, the CLI path uses `rpassword` directly.
pub async fn authenticate(
    handle: &mut Handle<SshHandler>,
    user: &str,
    host_name: &str,
    identity_files: &[PathBuf],
    identities_only: bool,
    cache: &mut PassphraseCache,
    auth_sender: Option<&SshAuthSender>,
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
                let secret = prompt_credential(prompt, auth_sender).await?;
                cache.insert(path.clone(), secret.clone());
                secret
            }
        };
        if try_pubkey(handle, user, path, Some(passphrase.as_str())).await? {
            return Ok(());
        }
    }

    // Step 3: password fallback (only if IdentitiesOnly is not set)
    if !identities_only {
        let prompt = password_prompt(user, host_name);
        let password = prompt_credential(prompt, auth_sender).await?;
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

/// Resolve a credential prompt to a `SecretString`, either via the TUI auth
/// bridge (when `sender` is `Some`) or via `rpassword` on the CLI path.
async fn prompt_credential(prompt: String, sender: Option<&SshAuthSender>) -> Result<SecretString> {
    match sender {
        Some(tx) => {
            let (responder, receiver) = tokio::sync::oneshot::channel::<String>();
            tx.send(SshAuthRequest { prompt, responder })
                .map_err(|_| anyhow::anyhow!("TUI auth bridge closed before prompt was sent"))?;
            let credential = receiver
                .await
                .context("TUI auth bridge dropped responder without replying")?;
            Ok(SecretString::new(credential))
        }
        None => {
            let pw = rpassword::prompt_password(&prompt)
                .with_context(|| format!("Failed to read credential: {prompt}"))?;
            Ok(SecretString::new(pw))
        }
    }
}

fn password_prompt(user: &str, host_name: &str) -> String {
    format!("{}@{} password: ", user, host_name)
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

    #[test]
    fn test_password_prompt_contains_real_hostname() {
        let prompt = password_prompt("alice", "web-prod-1");
        assert!(
            prompt.contains("alice"),
            "prompt should contain user: {prompt}"
        );
        assert!(
            prompt.contains("web-prod-1"),
            "prompt should contain resolved hostname: {prompt}",
        );
        assert!(
            !prompt.contains("<host>"),
            "prompt should not contain the literal placeholder: {prompt}",
        );
    }

    #[tokio::test]
    async fn test_auth_sender_path_returns_tui_supplied_credential() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SshAuthRequest>();
        let expected = "tui-supplied-passphrase";

        let responder_task = tokio::spawn(async move {
            let req = rx.recv().await.expect("auth request was sent");
            assert!(
                req.prompt.contains("/fake/key"),
                "prompt should name the key path: {}",
                req.prompt
            );
            req.responder
                .send(expected.to_string())
                .expect("responder accepted");
        });

        let secret = prompt_credential("Enter passphrase for /fake/key: ".to_string(), Some(&tx))
            .await
            .expect("TUI bridge path returns a SecretString");

        responder_task.await.unwrap();
        assert_eq!(secret.as_str(), expected);
    }

    #[tokio::test]
    async fn test_auth_sender_path_returns_error_when_bridge_drops_responder() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<SshAuthRequest>();

        let consumer = tokio::spawn(async move {
            // Receive the request then drop the responder without replying,
            // simulating a TUI popup cancel (Esc).
            let req = rx.recv().await.unwrap();
            drop(req.responder);
        });

        let result =
            prompt_credential("Enter passphrase for /fake/key: ".to_string(), Some(&tx)).await;

        consumer.await.unwrap();
        assert!(
            result.is_err(),
            "dropping the responder must surface as an error"
        );
    }
}

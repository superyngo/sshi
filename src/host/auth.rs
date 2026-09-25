//! SSH authentication chain: public key + password fallback with passphrase caching.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use russh::client::Handle;
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};
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
/// Auth chain (OpenSSH order, B20):
/// 1. ssh-agent keys via `SSH_AUTH_SOCK` (Unix). With `identities_only`, only
///    agent keys whose public half matches a listed identity's `.pub` file.
/// 2. Each identity file that loads without a passphrase.
/// 3. Each identity file that exists and is encrypted: prompt for its
///    passphrase (cached per path).
/// 4. Password prompt, unless `identities_only` is set.
///
/// `identity_files` already holds the OpenSSH default keys when no
/// `IdentityFile` is configured (`ParsedSshConfig::query`).
///
/// When `auth_sender` is `Some`, credential prompts are routed through the
/// TUI auth bridge (`SshAuthRequest` over the supplied mpsc, reply via the
/// embedded oneshot). When `None`, the CLI path uses `rpassword` directly.
#[allow(clippy::too_many_arguments)]
pub async fn authenticate(
    handle: &mut Handle<SshHandler>,
    user: &str,
    host_name: &str,
    identity_files: &[PathBuf],
    identities_only: bool,
    cache: &mut PassphraseCache,
    auth_sender: Option<&SshAuthSender>,
    timeout: Duration,
) -> Result<()> {
    // RSA keys need the server's preferred SHA-2 signature hash (rsa-sha2-*).
    let rsa_hash = net(timeout, handle.best_supported_rsa_hash())
        .await?
        .context("Public key authentication error")?
        .flatten();

    // Step 1: ssh-agent
    #[cfg(unix)]
    if try_agent(
        handle,
        user,
        identity_files,
        identities_only,
        rsa_hash,
        timeout,
    )
    .await?
    {
        return Ok(());
    }

    // Step 2: identity files that need no passphrase
    for path in identity_files {
        if let Ok(key) = russh::keys::load_secret_key(path, None) {
            if try_pubkey(handle, user, key, rsa_hash, timeout).await? {
                return Ok(());
            }
        }
    }

    // Step 3: encrypted identity files — prompt for the passphrase
    for path in identity_files.iter().filter(|p| is_encrypted_key(p)) {
        let passphrase = match cache.get(path) {
            Some(pp) => pp.clone(),
            None => {
                let prompt = format!("Enter passphrase for {}: ", path.display());
                let secret = prompt_credential(prompt, auth_sender).await?;
                cache.insert(path.clone(), secret.clone());
                secret
            }
        };
        if passphrase.as_str().is_empty() {
            continue;
        }
        if let Ok(key) = russh::keys::load_secret_key(path, Some(passphrase.as_str())) {
            if try_pubkey(handle, user, key, rsa_hash, timeout).await? {
                return Ok(());
            }
        }
    }

    // Step 4: password fallback (only if IdentitiesOnly is not set)
    if !identities_only {
        let prompt = password_prompt(user, host_name);
        let password = prompt_credential(prompt, auth_sender).await?;
        if net(
            timeout,
            handle.authenticate_password(user, password.as_str()),
        )
        .await?
        .context("Password authentication failed")?
        .success()
        {
            return Ok(());
        }
    }

    anyhow::bail!("All authentication methods exhausted for user '{}'", user)
}

/// Try every key held by the agent at `SSH_AUTH_SOCK`. Agent errors are
/// non-fatal: the chain continues with identity files.
#[cfg(unix)]
async fn try_agent(
    handle: &mut Handle<SshHandler>,
    user: &str,
    identity_files: &[PathBuf],
    identities_only: bool,
    rsa_hash: Option<russh::keys::HashAlg>,
    timeout: Duration,
) -> Result<bool> {
    use russh::keys::agent::{client::AgentClient, AgentIdentity};
    let Ok(mut agent) = AgentClient::connect_env().await else {
        return Ok(false);
    };
    let Ok(identities) = agent.request_identities().await else {
        return Ok(false);
    };
    let allowed: Vec<russh::keys::PublicKey> = identity_files
        .iter()
        .filter_map(|p| {
            let mut pub_path = p.clone().into_os_string();
            pub_path.push(".pub");
            russh::keys::PublicKey::read_openssh_file(Path::new(&pub_path)).ok()
        })
        .collect();
    for id in identities {
        let AgentIdentity::PublicKey { key, .. } = id else {
            continue;
        };
        if identities_only && !allowed.iter().any(|a| a.key_data() == key.key_data()) {
            continue;
        }
        let hash = if key.algorithm().is_rsa() {
            rsa_hash
        } else {
            None
        };
        match net(
            timeout,
            handle.authenticate_publickey_with(user, key, hash, &mut agent),
        )
        .await?
        {
            Ok(r) if r.success() => return Ok(true),
            Ok(_) => {}
            Err(e) => tracing::debug!("ssh-agent auth attempt failed: {e}"),
        }
    }
    Ok(false)
}

/// Whether `path` is a private key that needs a passphrase. Missing,
/// unreadable, or unparsable files are not, so they never trigger a prompt.
fn is_encrypted_key(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    if text.contains("Proc-Type: 4,ENCRYPTED") || text.contains("BEGIN ENCRYPTED PRIVATE KEY") {
        return true; // legacy PEM / PKCS#8
    }
    russh::keys::PrivateKey::from_openssh(&text)
        .map(|k| k.is_encrypted())
        .unwrap_or(false)
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

/// Bound one server round-trip of the auth exchange by `timeout`. Only
/// network calls are wrapped, so time spent at a credential prompt never
/// counts against it.
async fn net<F: std::future::Future>(timeout: Duration, fut: F) -> Result<F::Output> {
    tokio::time::timeout(timeout, fut).await.map_err(|_| {
        anyhow::anyhow!(
            "Authentication timed out (server did not answer within {}s)",
            timeout.as_secs()
        )
    })
}

/// Try public-key auth with a loaded key. Returns true if auth succeeded.
async fn try_pubkey(
    handle: &mut Handle<SshHandler>,
    user: &str,
    key: PrivateKey,
    rsa_hash: Option<russh::keys::HashAlg>,
    timeout: Duration,
) -> Result<bool> {
    let key = PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash);
    let authed = net(timeout, handle.authenticate_publickey(user, key))
        .await?
        .context("Public key authentication error")?;
    Ok(authed.success())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[cfg(unix)]
    #[test]
    fn is_encrypted_key_only_for_real_encrypted_keys() {
        let t = tempfile::tempdir().unwrap();
        let keygen = |name: &str, pw: &str| {
            let p = t.path().join(name);
            let ok = std::process::Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", pw, "-f"])
                .arg(&p)
                .status()
                .unwrap()
                .success();
            assert!(ok);
            p
        };
        let plain = keygen("plain", "");
        let enc = keygen("enc", "pw");
        let pem = t.path().join("pem");
        std::fs::write(
            &pem,
            "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\n",
        )
        .unwrap();

        assert!(!is_encrypted_key(&plain));
        assert!(is_encrypted_key(&enc));
        assert!(is_encrypted_key(&pem));
        assert!(!is_encrypted_key(&t.path().join("missing")));
    }

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

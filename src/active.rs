use crate::fsutil;
use crate::paths::Paths;
use crate::secret::Secret;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const CLAUDE_CODE_KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

pub trait ActiveStore {
    fn read(&self) -> Result<Secret>;
    fn write(&self, secret: &Secret) -> Result<()>;
}

/// Resolve the macOS Keychain account: an explicit `CCSWAP_KEYCHAIN_ACCOUNT`
/// override wins, then `$USER`. Empty values are treated as unset.
pub fn resolve_keychain_account(
    override_account: Option<String>,
    user: Option<String>,
) -> Result<String> {
    override_account
        .filter(|value| !value.is_empty())
        .or_else(|| user.filter(|value| !value.is_empty()))
        .context("set CCSWAP_KEYCHAIN_ACCOUNT or the USER environment variable")
}

#[derive(Debug, Clone)]
pub struct FileActiveStore {
    path: PathBuf,
}

impl FileActiveStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ActiveStore for FileActiveStore {
    fn read(&self) -> Result<Secret> {
        let bytes = fs::read(&self.path)
            .with_context(|| format!("read active credential {}", self.path.display()))?;
        Ok(Secret::new(bytes))
    }

    fn write(&self, secret: &Secret) -> Result<()> {
        fsutil::write_bytes_atomic(&self.path, secret.expose(), Some(0o600))
            .with_context(|| format!("write active credential {}", self.path.display()))
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct KeyringActiveStore {
    service: String,
    account: String,
}

#[cfg(target_os = "macos")]
impl KeyringActiveStore {
    pub fn from_env() -> Result<Self> {
        let account = resolve_keychain_account(
            std::env::var("CCSWAP_KEYCHAIN_ACCOUNT").ok(),
            std::env::var("USER").ok(),
        )?;
        Ok(Self::new(CLAUDE_CODE_KEYCHAIN_SERVICE, account))
    }

    pub fn new(service: impl Into<String>, account: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }
}

#[cfg(target_os = "macos")]
impl ActiveStore for KeyringActiveStore {
    fn read(&self) -> Result<Secret> {
        let entry = keyring::Entry::new(&self.service, &self.account)
            .with_context(|| format!("open Keychain item service='{}'", self.service))?;
        let bytes = entry
            .get_secret()
            .with_context(|| format!("read Keychain item service='{}'", self.service))?;
        Ok(Secret::new(bytes))
    }

    fn write(&self, secret: &Secret) -> Result<()> {
        let entry = keyring::Entry::new(&self.service, &self.account)
            .with_context(|| format!("open Keychain item service='{}'", self.service))?;
        entry
            .set_secret(secret.expose())
            .with_context(|| format!("write Keychain item service='{}'", self.service))
    }
}

#[cfg(target_os = "macos")]
pub type SystemActiveStore = KeyringActiveStore;

#[cfg(target_os = "macos")]
pub fn system_active_store(_paths: &Paths) -> Result<SystemActiveStore> {
    KeyringActiveStore::from_env()
}

#[cfg(target_os = "linux")]
pub type SystemActiveStore = FileActiveStore;

#[cfg(target_os = "linux")]
pub fn system_active_store(paths: &Paths) -> Result<SystemActiveStore> {
    Ok(FileActiveStore::new(paths.linux_credentials_path.clone()))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub type SystemActiveStore = FileActiveStore;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn system_active_store(paths: &Paths) -> Result<SystemActiveStore> {
    Ok(FileActiveStore::new(paths.linux_credentials_path.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    fn keychain_account_prefers_override_then_user() {
        assert_eq!(
            resolve_keychain_account(Some("acct".into()), Some("user".into())).unwrap(),
            "acct"
        );
        assert_eq!(
            resolve_keychain_account(None, Some("user".into())).unwrap(),
            "user"
        );
        assert_eq!(
            resolve_keychain_account(Some(String::new()), Some("user".into())).unwrap(),
            "user",
            "empty override is treated as unset"
        );
        assert!(resolve_keychain_account(None, None).is_err());
        assert!(resolve_keychain_account(Some(String::new()), Some(String::new())).is_err());
    }

    #[test]
    fn file_active_store_round_trips_secret() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        let store = FileActiveStore::new(&path);

        store.write(&Secret::new(b"token-json".to_vec())).unwrap();
        let loaded = store.read().unwrap();

        assert_eq!(loaded.expose(), b"token-json");
    }

    #[cfg(unix)]
    #[test]
    fn file_active_store_writes_credential_0600() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".credentials.json");
        let store = FileActiveStore::new(&path);

        store.write(&Secret::new(b"token-json".to_vec())).unwrap();

        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches the real macOS Keychain"]
    fn macos_keychain_active_store_round_trips_secret() {
        let account = format!("ccswap-test-{}", std::process::id());
        let store = KeyringActiveStore::new("ccswap-test-active", &account);

        store.write(&Secret::new(b"token-json".to_vec())).unwrap();
        let loaded = store.read().unwrap();
        keyring::Entry::new("ccswap-test-active", &account)
            .unwrap()
            .delete_credential()
            .unwrap();

        assert_eq!(loaded.expose(), b"token-json");
    }
}

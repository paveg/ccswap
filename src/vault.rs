use crate::fsutil;
use crate::paths::Paths;
use crate::profile::validate_storage_key;
use crate::secret::Secret;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub const CCSWAP_KEYCHAIN_SERVICE: &str = "ccswap";

pub trait ProfileVault {
    fn store(&self, name: &str, secret: &Secret) -> Result<()>;
    fn load(&self, name: &str) -> Result<Secret>;
    fn delete(&self, name: &str) -> Result<()>;
}

#[derive(Debug, Clone)]
pub struct FileVault {
    dir: PathBuf,
}

impl FileVault {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path_for(&self, name: &str) -> Result<PathBuf> {
        validate_storage_key(name)?;
        Ok(self.dir.join(format!("{name}.secret")))
    }
}

impl ProfileVault for FileVault {
    fn store(&self, name: &str, secret: &Secret) -> Result<()> {
        let path = self.path_for(name)?;
        fsutil::ensure_private_dir(&self.dir)?;
        fsutil::write_bytes_atomic(&path, secret.expose(), Some(0o600))
            .with_context(|| format!("write profile secret {}", path.display()))
    }

    fn load(&self, name: &str) -> Result<Secret> {
        let path = self.path_for(name)?;
        let bytes =
            fs::read(&path).with_context(|| format!("read profile secret {}", path.display()))?;
        Ok(Secret::new(bytes))
    }

    fn delete(&self, name: &str) -> Result<()> {
        let path = self.path_for(name)?;
        fsutil::remove_file_if_exists(&path)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Debug, Clone)]
pub struct KeyringVault {
    service: String,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl KeyringVault {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, name: &str) -> Result<keyring::Entry> {
        validate_storage_key(name)?;
        keyring::Entry::new(&self.service, name)
            .with_context(|| format!("open keyring profile '{name}'"))
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Default for KeyringVault {
    fn default() -> Self {
        Self::new(CCSWAP_KEYCHAIN_SERVICE)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl ProfileVault for KeyringVault {
    fn store(&self, name: &str, secret: &Secret) -> Result<()> {
        self.entry(name)?
            .set_secret(secret.expose())
            .with_context(|| format!("write keyring profile '{name}'"))
    }

    fn load(&self, name: &str) -> Result<Secret> {
        let bytes = self
            .entry(name)?
            .get_secret()
            .with_context(|| format!("read keyring profile '{name}'"))?;
        Ok(Secret::new(bytes))
    }

    fn delete(&self, name: &str) -> Result<()> {
        match self.entry(name)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err).with_context(|| format!("delete keyring profile '{name}'")),
        }
    }
}

#[cfg(target_os = "macos")]
pub type SystemProfileVault = KeyringVault;

#[cfg(target_os = "macos")]
pub fn system_profile_vault(_paths: &Paths) -> Result<SystemProfileVault> {
    Ok(KeyringVault::default())
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
pub struct LinuxProfileVault {
    keyring: KeyringVault,
    fallback: FileVault,
}

#[cfg(target_os = "linux")]
impl LinuxProfileVault {
    pub fn new(fallback_dir: impl Into<PathBuf>) -> Self {
        Self {
            keyring: KeyringVault::default(),
            fallback: FileVault::new(fallback_dir),
        }
    }
}

#[cfg(target_os = "linux")]
impl ProfileVault for LinuxProfileVault {
    fn store(&self, name: &str, secret: &Secret) -> Result<()> {
        match self.keyring.store(name, secret) {
            Ok(()) => Ok(()),
            Err(_) => self.fallback.store(name, secret),
        }
    }

    fn load(&self, name: &str) -> Result<Secret> {
        match self.keyring.load(name) {
            Ok(secret) => Ok(secret),
            Err(keyring_err) => self
                .fallback
                .load(name)
                .with_context(|| format!("keyring load failed: {keyring_err:#}")),
        }
    }

    fn delete(&self, name: &str) -> Result<()> {
        let keyring_result = self.keyring.delete(name);
        let fallback_result = self.fallback.delete(name);

        match (keyring_result, fallback_result) {
            (Ok(()), Ok(())) | (Ok(()), Err(_)) | (Err(_), Ok(())) => Ok(()),
            (Err(keyring_err), Err(fallback_err)) => Err(fallback_err)
                .with_context(|| format!("keyring delete failed: {keyring_err:#}")),
        }
    }
}

#[cfg(target_os = "linux")]
pub type SystemProfileVault = LinuxProfileVault;

#[cfg(target_os = "linux")]
pub fn system_profile_vault(paths: &Paths) -> Result<SystemProfileVault> {
    Ok(LinuxProfileVault::new(paths.file_vault_dir.clone()))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub type SystemProfileVault = FileVault;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn system_profile_vault(paths: &Paths) -> Result<SystemProfileVault> {
    Ok(FileVault::new(paths.file_vault_dir.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    fn file_vault_round_trips_secret() {
        let dir = tempdir().unwrap();
        let vault = FileVault::new(dir.path());

        vault
            .store("work", &Secret::new(b"token-json".to_vec()))
            .unwrap();
        let loaded = vault.load("work").unwrap();

        assert_eq!(loaded.expose(), b"token-json");
    }

    #[cfg(unix)]
    #[test]
    fn file_vault_writes_secret_0600() {
        let dir = tempdir().unwrap();
        let vault = FileVault::new(dir.path());

        vault
            .store("work", &Secret::new(b"token-json".to_vec()))
            .unwrap();

        let path = vault.path_for("work").unwrap();
        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn file_vault_delete_ignores_missing_secret() {
        let dir = tempdir().unwrap();
        let vault = FileVault::new(dir.path());

        vault.delete("work").unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches the real macOS Keychain"]
    fn macos_keyring_vault_round_trips_secret() {
        let name = format!("ccswap-test-{}", std::process::id());
        let vault = KeyringVault::new("ccswap-test-vault");

        vault
            .store(&name, &Secret::new(b"token-json".to_vec()))
            .unwrap();
        let loaded = vault.load(&name).unwrap();
        vault.delete(&name).unwrap();

        assert_eq!(loaded.expose(), b"token-json");
    }
}

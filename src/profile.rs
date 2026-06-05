use crate::fsutil;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub const RESERVED_PREVIOUS_NAME: &str = "__previous";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountKey {
    pub account_uuid: String,
    pub organization_uuid: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub oauth_account: Value,
}

#[derive(Debug, Clone)]
pub struct ProfileRegistry {
    dir: PathBuf,
}

impl ProfileRegistry {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save(&self, name: &str, oauth_account: &Value) -> Result<()> {
        validate_profile_name(name)?;
        ensure_oauth_account_object(oauth_account)?;

        let path = self.path_for(name)?;
        fsutil::ensure_private_dir(&self.dir)?;
        fsutil::write_json_atomic(&path, oauth_account, Some(0o600))
            .with_context(|| format!("write profile metadata {}", path.display()))
    }

    pub fn load(&self, name: &str) -> Result<Value> {
        validate_profile_name(name)?;
        let path = self.path_for(name)?;
        let bytes = fs::read(&path).with_context(|| format!("read profile {}", path.display()))?;
        let value: Value =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        ensure_oauth_account_object(&value)?;
        Ok(value)
    }

    pub fn list(&self) -> Result<Vec<Profile>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut profiles = Vec::new();
        for entry in
            fs::read_dir(&self.dir).with_context(|| format!("read {}", self.dir.display()))?
        {
            let entry = entry.with_context(|| format!("read entry in {}", self.dir.display()))?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let Some(name) = profile_name_from_path(&path) else {
                continue;
            };
            if validate_profile_name(&name).is_err() {
                continue;
            }
            let Ok(oauth_account) = self.load(&name) else {
                continue;
            };
            profiles.push(Profile {
                name,
                oauth_account,
            });
        }

        profiles.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(profiles)
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        validate_profile_name(name)?;
        let path = self.path_for(name)?;
        if !path.exists() {
            bail!("profile '{name}' does not exist");
        }
        fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))
    }

    pub fn exists(&self, name: &str) -> Result<bool> {
        validate_profile_name(name)?;
        Ok(self.path_for(name)?.exists())
    }

    pub fn find_by_account(&self, oauth_account: &Value) -> Result<Option<Profile>> {
        let wanted = account_key(oauth_account)?;
        for profile in self.list()? {
            if account_key(&profile.oauth_account).ok() == Some(wanted.clone()) {
                return Ok(Some(profile));
            }
        }
        Ok(None)
    }

    pub fn path_for(&self, name: &str) -> Result<PathBuf> {
        validate_profile_name(name)?;
        Ok(self.dir.join(format!("{name}.json")))
    }
}

pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("profile name cannot be empty");
    }
    if matches!(name, "." | ".." | "-" | RESERVED_PREVIOUS_NAME) {
        bail!("profile name '{name}' is reserved");
    }
    if !name.bytes().all(|byte| {
        matches!(
            byte,
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b'-'
        )
    }) {
        bail!("profile name may contain only ASCII letters, numbers, '.', '_' and '-'");
    }
    Ok(())
}

/// Validate a secret-store key. Like [`validate_profile_name`] but also admits
/// the internal `__previous` slot, which user-facing commands must keep rejecting.
/// Path-traversal safety (no `.`/`..`/separators/empty) is preserved.
pub fn validate_storage_key(name: &str) -> Result<()> {
    if name == RESERVED_PREVIOUS_NAME {
        return Ok(());
    }
    validate_profile_name(name)
}

pub fn account_key(oauth_account: &Value) -> Result<AccountKey> {
    ensure_oauth_account_object(oauth_account)?;
    let account_uuid = required_string(oauth_account, "accountUuid")?;
    let organization_uuid = required_string(oauth_account, "organizationUuid")?;
    Ok(AccountKey {
        account_uuid,
        organization_uuid,
    })
}

pub fn identity_summary(oauth_account: &Value) -> String {
    let email = string_field(oauth_account, "emailAddress").unwrap_or("unknown email");
    match string_field(oauth_account, "organizationName") {
        Some(org) if !org.is_empty() => format!("{email} / {org}"),
        _ => email.to_string(),
    }
}

fn ensure_oauth_account_object(value: &Value) -> Result<()> {
    if !value.is_object() {
        bail!("oauthAccount snapshot must be a JSON object");
    }
    Ok(())
}

fn required_string(value: &Value, key: &str) -> Result<String> {
    string_field(value, key)
        .map(str::to_string)
        .with_context(|| format!("oauthAccount.{key} must be a string"))
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn profile_name_from_path(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    let name = file_name.strip_suffix(".json")?;
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    fn account(account_uuid: &str, organization_uuid: &str) -> Value {
        json!({
            "accountUuid": account_uuid,
            "organizationUuid": organization_uuid,
            "emailAddress": "user@example.com",
            "organizationName": "Org"
        })
    }

    #[test]
    fn rejects_path_like_profile_names() {
        for name in [
            "",
            ".",
            "..",
            "-",
            "__previous",
            "../x",
            "x/y",
            "x\\y",
            "space name",
        ] {
            assert!(validate_profile_name(name).is_err(), "{name}");
        }

        for name in ["work", "personal-1", "org.alpha", "org_beta"] {
            validate_profile_name(name).unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn save_creates_profiles_dir_private_0700() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let profiles_dir = dir.path().join("profiles");
        let registry = ProfileRegistry::new(&profiles_dir);

        registry.save("work", &account("a", "o")).unwrap();

        let mode = fs::metadata(&profiles_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn storage_key_allows_previous_slot_but_user_name_does_not() {
        assert!(validate_storage_key(RESERVED_PREVIOUS_NAME).is_ok());
        assert!(validate_profile_name(RESERVED_PREVIOUS_NAME).is_err());
        for bad in ["", "..", "../x", "x/y", "."] {
            assert!(validate_storage_key(bad).is_err(), "{bad}");
        }
        validate_storage_key("work").unwrap();
    }

    #[test]
    fn saves_and_loads_oauth_account_metadata() {
        let dir = tempdir().unwrap();
        let registry = ProfileRegistry::new(dir.path());
        let oauth_account = account("a", "o");

        registry.save("work", &oauth_account).unwrap();
        let loaded = registry.load("work").unwrap();

        assert_eq!(loaded, oauth_account);
    }

    #[test]
    fn metadata_file_does_not_receive_secret_bytes() {
        let dir = tempdir().unwrap();
        let registry = ProfileRegistry::new(dir.path());
        registry.save("work", &account("a", "o")).unwrap();

        let path = registry.path_for("work").unwrap();
        let metadata = fs::read_to_string(path).unwrap();

        assert!(!metadata.contains("super-secret-token"));
        assert!(!metadata.contains("oauthAccount"));
        assert!(metadata.contains("accountUuid"));
    }

    #[test]
    fn list_skips_malformed_and_foreign_files() {
        let dir = tempdir().unwrap();
        let registry = ProfileRegistry::new(dir.path());
        registry.save("good", &account("a", "o")).unwrap();
        fs::write(dir.path().join("malformed.json"), b"{ not json").unwrap();
        fs::write(dir.path().join("nonobject.json"), b"[1,2,3]").unwrap();
        fs::write(dir.path().join("bad name.json"), b"{}").unwrap();

        let names: Vec<String> = registry
            .list()
            .unwrap()
            .into_iter()
            .map(|profile| profile.name)
            .collect();

        assert_eq!(names, vec!["good"]);
    }

    #[test]
    fn lists_profiles_sorted() {
        let dir = tempdir().unwrap();
        let registry = ProfileRegistry::new(dir.path());
        registry.save("zeta", &account("z", "o")).unwrap();
        registry.save("alpha", &account("a", "o")).unwrap();

        let names: Vec<String> = registry
            .list()
            .unwrap()
            .into_iter()
            .map(|profile| profile.name)
            .collect();

        assert_eq!(names, vec!["alpha", "zeta"]);
    }

    #[test]
    fn current_matching_uses_account_and_organization() {
        let dir = tempdir().unwrap();
        let registry = ProfileRegistry::new(dir.path());
        registry.save("org-a", &account("acct", "a")).unwrap();
        registry.save("org-b", &account("acct", "b")).unwrap();

        let found = registry
            .find_by_account(&account("acct", "b"))
            .unwrap()
            .unwrap();

        assert_eq!(found.name, "org-b");
    }

    #[test]
    fn account_key_requires_both_uuid_fields() {
        assert!(account_key(&json!({ "accountUuid": "a" })).is_err());
        assert!(account_key(&json!({ "organizationUuid": "o" })).is_err());
        assert!(account_key(&json!({ "accountUuid": "a", "organizationUuid": "o" })).is_ok());
    }
}

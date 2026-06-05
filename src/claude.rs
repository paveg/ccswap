use crate::fsutil;
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ClaudeFile {
    path: PathBuf,
}

impl ClaudeFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read_root(&self) -> Result<Value> {
        let bytes =
            fs::read(&self.path).with_context(|| format!("read {}", self.path.display()))?;
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", self.path.display()))
    }

    pub fn read_oauth_account(&self) -> Result<Value> {
        oauth_account_from_root(&self.read_root()?)
    }

    pub fn write_oauth_account(&self, new_account: &Value) -> Result<()> {
        let root = self.read_root()?;
        let updated = replace_oauth_account(root, new_account.clone())?;
        let mode = fsutil::existing_mode(&self.path, 0o600);
        fsutil::write_json_atomic(&self.path, &updated, Some(mode))
            .with_context(|| format!("write {}", self.path.display()))
    }
}

pub fn oauth_account_from_root(root: &Value) -> Result<Value> {
    let account = root
        .get("oauthAccount")
        .context("~/.claude.json has no oauthAccount object")?;
    if !account.is_object() {
        bail!("oauthAccount must be a JSON object");
    }
    Ok(account.clone())
}

/// Replace only the `oauthAccount` key in a parsed `~/.claude.json` value,
/// preserving every other key. The input must be a JSON object.
pub fn replace_oauth_account(mut root: Value, new_account: Value) -> Result<Value> {
    if !new_account.is_object() {
        bail!("oauthAccount snapshot must be a JSON object");
    }
    match root.as_object_mut() {
        Some(map) => {
            map.insert("oauthAccount".to_string(), new_account);
            Ok(root)
        }
        None => bail!("~/.claude.json root is not a JSON object"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    fn replaces_only_oauth_account_preserving_other_keys() {
        let root = json!({
            "numStartups": 42,
            "oauthAccount": { "accountUuid": "old", "emailAddress": "old@example.com" },
            "projects": { "a": 1 }
        });
        let new = json!({ "accountUuid": "new", "emailAddress": "new@example.com" });

        let out = replace_oauth_account(root, new.clone()).unwrap();

        assert_eq!(out["oauthAccount"], new);
        assert_eq!(out["numStartups"], 42);
        assert_eq!(out["projects"]["a"], 1);
    }

    #[test]
    fn inserts_oauth_account_when_missing() {
        let root = json!({ "numStartups": 1 });
        let out = replace_oauth_account(root, json!({ "accountUuid": "x" })).unwrap();
        assert_eq!(out["oauthAccount"]["accountUuid"], "x");
        assert_eq!(out["numStartups"], 1);
    }

    #[test]
    fn accepts_empty_root_object() {
        let out = replace_oauth_account(json!({}), json!({ "accountUuid": "x" })).unwrap();
        assert_eq!(out["oauthAccount"]["accountUuid"], "x");
    }

    #[test]
    fn preserves_original_key_order_and_position() {
        let root = json!({ "zebra": 1, "oauthAccount": { "accountUuid": "old" }, "apple": 2 });
        let out = replace_oauth_account(root, json!({ "accountUuid": "new" })).unwrap();
        let keys: Vec<&str> = out
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["zebra", "oauthAccount", "apple"]);
    }

    #[test]
    fn rejects_non_object_root() {
        for root in [
            json!([1, 2, 3]),
            json!("s"),
            json!(7),
            json!(true),
            json!(null),
        ] {
            assert!(replace_oauth_account(root, json!({ "accountUuid": "x" })).is_err());
        }
    }

    #[test]
    fn rejects_non_object_new_account() {
        for acc in [json!(null), json!("tok"), json!([1]), json!(7), json!(true)] {
            assert!(replace_oauth_account(json!({}), acc).is_err());
        }
    }

    #[test]
    fn reads_oauth_account_from_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        fs::write(
            &path,
            r#"{"oauthAccount":{"accountUuid":"a","organizationUuid":"o"},"x":1}"#,
        )
        .unwrap();

        let file = ClaudeFile::new(&path);
        let account = file.read_oauth_account().unwrap();

        assert_eq!(account["accountUuid"], "a");
        assert_eq!(account["organizationUuid"], "o");
    }

    #[test]
    fn writes_oauth_account_atomically_preserving_other_keys() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        fs::write(
            &path,
            r#"{"numStartups":42,"oauthAccount":{"accountUuid":"old"},"projects":{"a":1}}"#,
        )
        .unwrap();

        let file = ClaudeFile::new(&path);
        file.write_oauth_account(&json!({
            "accountUuid": "new",
            "organizationUuid": "org"
        }))
        .unwrap();

        let out: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(out["oauthAccount"]["accountUuid"], "new");
        assert_eq!(out["oauthAccount"]["organizationUuid"], "org");
        assert_eq!(out["numStartups"], 42);
        assert_eq!(out["projects"]["a"], 1);
    }

    #[cfg(unix)]
    #[test]
    fn write_oauth_account_preserves_existing_file_mode() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        fs::write(&path, r#"{"oauthAccount":{"accountUuid":"old"}}"#).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        let file = ClaudeFile::new(&path);
        file.write_oauth_account(&json!({ "accountUuid": "new" }))
            .unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }
}

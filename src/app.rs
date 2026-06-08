use crate::active::{system_active_store, ActiveStore, SystemActiveStore};
use crate::claude::ClaudeFile;
use crate::fsutil;
use crate::hooks::{ConfiguredHooks, HookPhase, NoopHooks, SwitchHookContext, SwitchHooks};
use crate::paths::Paths;
use crate::profile::{
    account_key, identity_summary, validate_profile_name, Profile, ProfileRegistry,
    RESERVED_PREVIOUS_NAME,
};
use crate::secret::Secret;
use crate::vault::{system_profile_vault, ProfileVault, SystemProfileVault};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Ccswap<A, V, H = NoopHooks> {
    active: A,
    vault: V,
    hooks: H,
    claude: ClaudeFile,
    profiles: ProfileRegistry,
    previous: PreviousStore,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SaveResult {
    pub name: String,
    pub oauth_account: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UseResult {
    pub name: String,
    pub oauth_account: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CurrentResult {
    pub name: Option<String>,
    pub oauth_account: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListEntry {
    pub name: String,
    pub oauth_account: Value,
    pub current: bool,
}

#[derive(Debug, Clone)]
pub struct PreviousStore {
    path: PathBuf,
}

impl PreviousStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn save(&self, oauth_account: &Value) -> Result<()> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fsutil::ensure_private_dir(parent)?;
        }
        fsutil::write_json_atomic(&self.path, oauth_account, Some(0o600))
            .with_context(|| format!("write previous account {}", self.path.display()))
    }

    pub fn load(&self) -> Result<Value> {
        let bytes = std::fs::read(&self.path)
            .with_context(|| format!("read previous account {}", self.path.display()))?;
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", self.path.display()))
    }
}

impl Ccswap<SystemActiveStore, SystemProfileVault, ConfiguredHooks> {
    pub fn discover() -> Result<Self> {
        let paths = Paths::discover()?;
        let active = system_active_store(&paths)?;
        let vault = system_profile_vault(&paths)?;
        let hooks = ConfiguredHooks::new(paths.hooks_path.clone());
        Ok(Self::from_parts_with_hooks(paths, active, vault, hooks))
    }
}

impl<A, V> Ccswap<A, V, NoopHooks>
where
    A: ActiveStore,
    V: ProfileVault,
{
    pub fn from_parts(paths: Paths, active: A, vault: V) -> Self {
        Self::from_parts_with_hooks(paths, active, vault, NoopHooks)
    }

    pub fn new(
        active: A,
        vault: V,
        claude: ClaudeFile,
        profiles: ProfileRegistry,
        previous: PreviousStore,
    ) -> Self {
        Self::new_with_hooks(active, vault, claude, profiles, previous, NoopHooks)
    }
}

impl<A, V, H> Ccswap<A, V, H>
where
    A: ActiveStore,
    V: ProfileVault,
    H: SwitchHooks,
{
    pub fn from_parts_with_hooks(paths: Paths, active: A, vault: V, hooks: H) -> Self {
        let claude = ClaudeFile::new(paths.claude_json_path);
        let profiles = ProfileRegistry::new(paths.profiles_dir);
        let previous = PreviousStore::new(paths.previous_path);
        Self::new_with_hooks(active, vault, claude, profiles, previous, hooks)
    }

    pub fn new_with_hooks(
        active: A,
        vault: V,
        claude: ClaudeFile,
        profiles: ProfileRegistry,
        previous: PreviousStore,
        hooks: H,
    ) -> Self {
        Self {
            active,
            vault,
            hooks,
            claude,
            profiles,
            previous,
        }
    }

    pub fn save_profile(&self, name: &str) -> Result<SaveResult> {
        validate_profile_name(name)?;
        let oauth_account = self
            .claude
            .read_oauth_account()
            .context("read active oauthAccount")?;
        let secret = self.active.read().context("read active credential")?;

        self.vault
            .store(name, &secret)
            .with_context(|| format!("save credential for profile '{name}'"))?;

        if let Err(err) = self.profiles.save(name, &oauth_account) {
            let _ = self.vault.delete(name);
            return Err(err).with_context(|| format!("save metadata for profile '{name}'"));
        }

        Ok(SaveResult {
            name: name.to_string(),
            oauth_account,
        })
    }

    pub fn use_profile(&self, name: &str) -> Result<UseResult> {
        validate_profile_name(name)?;
        let target_account = self
            .profiles
            .load(name)
            .with_context(|| format!("load profile '{name}' metadata"))?;
        let target_secret = self
            .vault
            .load(name)
            .with_context(|| format!("load profile '{name}' credential"))?;
        self.switch_to(name.to_string(), target_account, &target_secret)
    }

    pub fn use_previous(&self) -> Result<UseResult> {
        let target_account = self
            .previous
            .load()
            .context("no previous account to return to")?;
        let target_secret = self
            .vault
            .load(RESERVED_PREVIOUS_NAME)
            .context("no previous credential to return to")?;
        let name = self
            .profiles
            .find_by_account(&target_account)
            .ok()
            .flatten()
            .map(|profile| profile.name)
            .unwrap_or_else(|| "-".to_string());
        self.switch_to(name, target_account, &target_secret)
    }

    /// Snapshot the current account+token into the previous slot, then swap C
    /// (oauthAccount) and A (active credential) to the target, rolling back on
    /// any failure.
    fn switch_to(
        &self,
        name: String,
        target_account: Value,
        target_secret: &Secret,
    ) -> Result<UseResult> {
        let previous_account = self
            .claude
            .read_oauth_account()
            .context("snapshot current oauthAccount")?;
        let previous_profile = self.profile_name_for_account(&previous_account)?;
        let hook_context = SwitchHookContext::new(
            name.clone(),
            target_account.clone(),
            previous_profile,
            previous_account.clone(),
        );
        self.hooks
            .run(HookPhase::PreUse, &hook_context)
            .context("pre-use hook failed")?;

        let previous_secret = self.active.read().context("snapshot current credential")?;

        self.previous.save(&previous_account)?;
        self.vault
            .store(RESERVED_PREVIOUS_NAME, &previous_secret)
            .context("snapshot current credential to previous slot")?;

        if let Err(err) = self.claude.write_oauth_account(&target_account) {
            return self.switch_error_with_rollback(
                err,
                &previous_account,
                &previous_secret,
                &hook_context,
            );
        }

        if let Err(err) = self.active.write(target_secret) {
            return self.switch_error_with_rollback(
                err,
                &previous_account,
                &previous_secret,
                &hook_context,
            );
        }

        if let Err(err) = self.hooks.run(HookPhase::PostUse, &hook_context) {
            return self.switch_error_with_rollback(
                err.context("post-use hook failed"),
                &previous_account,
                &previous_secret,
                &hook_context,
            );
        }

        Ok(UseResult {
            name,
            oauth_account: target_account,
        })
    }

    pub fn list_profiles(&self) -> Result<Vec<ListEntry>> {
        let current_key = self
            .claude
            .read_oauth_account()
            .ok()
            .and_then(|account| account_key(&account).ok());

        let mut entries = Vec::new();
        for profile in self.profiles.list()? {
            let current = current_key
                .as_ref()
                .is_some_and(|key| account_key(&profile.oauth_account).ok().as_ref() == Some(key));
            entries.push(ListEntry {
                name: profile.name,
                oauth_account: profile.oauth_account,
                current,
            });
        }
        Ok(entries)
    }

    pub fn current_profile(&self) -> Result<CurrentResult> {
        let oauth_account = self.claude.read_oauth_account()?;
        let name = self
            .profiles
            .find_by_account(&oauth_account)?
            .map(|profile: Profile| profile.name);
        Ok(CurrentResult {
            name,
            oauth_account,
        })
    }

    pub fn remove_profile(&self, name: &str) -> Result<()> {
        validate_profile_name(name)?;
        if !self.profiles.exists(name)? {
            anyhow::bail!("profile '{name}' does not exist");
        }
        self.vault
            .delete(name)
            .with_context(|| format!("delete credential for profile '{name}'"))?;
        self.profiles.remove(name)
    }

    fn switch_error_with_rollback(
        &self,
        err: anyhow::Error,
        previous_account: &Value,
        previous_secret: &Secret,
        hook_context: &SwitchHookContext,
    ) -> Result<UseResult> {
        match self.rollback(previous_account, previous_secret) {
            Ok(()) => match self.run_post_use_rollback_hook(hook_context) {
                Ok(()) => Err(err.context("switch failed; rolled back to previous account")),
                Err(hook_err) => Err(anyhow!(
                    "switch failed: {err:#}; rolled back to previous account; rollback post-use hook failed: {hook_err:#}"
                )),
            },
            Err(rollback_err) => Err(anyhow!(
                "switch failed: {err:#}; rollback failed: {rollback_err:#}"
            )),
        }
    }

    fn run_post_use_rollback_hook(&self, hook_context: &SwitchHookContext) -> Result<()> {
        let Some(rollback_context) = hook_context.previous_as_rollback_target() else {
            return Ok(());
        };
        self.hooks.run(HookPhase::PostUse, &rollback_context)
    }

    fn rollback(&self, previous_account: &Value, previous_secret: &Secret) -> Result<()> {
        self.claude
            .write_oauth_account(previous_account)
            .context("rollback oauthAccount")?;
        self.active
            .write(previous_secret)
            .context("rollback credential")
    }

    fn profile_name_for_account(&self, oauth_account: &Value) -> Result<Option<String>> {
        Ok(self
            .profiles
            .find_by_account(oauth_account)?
            .map(|profile| profile.name))
    }
}

pub fn format_identity(oauth_account: &Value) -> String {
    identity_summary(oauth_account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::active::ActiveStore;
    use crate::hooks::{HookPhase, SwitchHookContext, SwitchHooks};
    use crate::profile::ProfileRegistry;
    use crate::vault::{FileVault, ProfileVault};
    use anyhow::{bail, Result};
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::fs;
    use std::rc::Rc;
    use tempfile::tempdir;

    #[derive(Debug)]
    struct MemoryActiveStore {
        secret: RefCell<Secret>,
        fail_next_write: Cell<bool>,
    }

    impl MemoryActiveStore {
        fn new(secret: Secret) -> Self {
            Self {
                secret: RefCell::new(secret),
                fail_next_write: Cell::new(false),
            }
        }

        fn fail_next_write(&self) {
            self.fail_next_write.set(true);
        }
    }

    impl ActiveStore for MemoryActiveStore {
        fn read(&self) -> Result<Secret> {
            Ok(self.secret.borrow().clone())
        }

        fn write(&self, secret: &Secret) -> Result<()> {
            if self.fail_next_write.replace(false) {
                bail!("active write failed")
            }
            *self.secret.borrow_mut() = secret.clone();
            Ok(())
        }
    }

    #[derive(Debug, Default, Clone)]
    struct RecordingHooks {
        calls: Rc<RefCell<Vec<(HookPhase, String)>>>,
        failures: Rc<RefCell<Vec<(HookPhase, String)>>>,
    }

    impl RecordingHooks {
        fn fail_for(&self, phase: HookPhase, target_profile: &str) {
            self.failures
                .borrow_mut()
                .push((phase, target_profile.to_string()));
        }

        fn calls(&self) -> Vec<(HookPhase, String)> {
            self.calls.borrow().clone()
        }
    }

    impl SwitchHooks for RecordingHooks {
        fn run(&self, phase: HookPhase, context: &SwitchHookContext) -> Result<()> {
            self.calls
                .borrow_mut()
                .push((phase, context.target_profile.clone()));
            if self
                .failures
                .borrow()
                .contains(&(phase, context.target_profile.clone()))
            {
                bail!("hook failed for {}", context.target_profile);
            }
            Ok(())
        }
    }

    fn account(account_uuid: &str, organization_uuid: &str) -> Value {
        json!({
            "accountUuid": account_uuid,
            "organizationUuid": organization_uuid,
            "emailAddress": format!("{account_uuid}@example.com"),
            "organizationName": organization_uuid
        })
    }

    fn test_app() -> (
        Ccswap<MemoryActiveStore, FileVault>,
        tempfile::TempDir,
        PathBuf,
    ) {
        let dir = tempdir().unwrap();
        let claude_path = dir.path().join(".claude.json");
        fs::write(
            &claude_path,
            serde_json::to_vec(&json!({
                "oauthAccount": account("old", "org"),
                "projects": { "keep": true }
            }))
            .unwrap(),
        )
        .unwrap();

        let active = MemoryActiveStore::new(Secret::new(b"old-token".to_vec()));
        let vault = FileVault::new(dir.path().join("vault"));
        let app = Ccswap::new(
            active,
            vault,
            ClaudeFile::new(&claude_path),
            ProfileRegistry::new(dir.path().join("profiles")),
            PreviousStore::new(dir.path().join("state").join("previous.json")),
        );

        (app, dir, claude_path)
    }

    fn test_app_with_hooks(
        hooks: RecordingHooks,
    ) -> (
        Ccswap<MemoryActiveStore, FileVault, RecordingHooks>,
        tempfile::TempDir,
        PathBuf,
    ) {
        let dir = tempdir().unwrap();
        let claude_path = dir.path().join(".claude.json");
        fs::write(
            &claude_path,
            serde_json::to_vec(&json!({
                "oauthAccount": account("old", "org"),
                "projects": { "keep": true }
            }))
            .unwrap(),
        )
        .unwrap();

        let active = MemoryActiveStore::new(Secret::new(b"old-token".to_vec()));
        let vault = FileVault::new(dir.path().join("vault"));
        let app = Ccswap::new_with_hooks(
            active,
            vault,
            ClaudeFile::new(&claude_path),
            ProfileRegistry::new(dir.path().join("profiles")),
            PreviousStore::new(dir.path().join("state").join("previous.json")),
            hooks,
        );

        (app, dir, claude_path)
    }

    #[cfg(unix)]
    #[test]
    fn previous_store_creates_state_dir_private_0700() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let state_dir = dir.path().join("state");
        let store = PreviousStore::new(state_dir.join("previous.json"));

        store.save(&account("a", "o")).unwrap();

        let mode = fs::metadata(&state_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn save_profile_stores_secret_in_vault_and_metadata_in_profile_file() {
        let (app, _dir, _claude_path) = test_app();

        let result = app.save_profile("work").unwrap();
        let saved_secret = app.vault.load("work").unwrap();
        let saved_account = app.profiles.load("work").unwrap();

        assert_eq!(result.name, "work");
        assert_eq!(saved_secret.expose(), b"old-token");
        assert_eq!(saved_account["accountUuid"], "old");
    }

    #[test]
    fn list_marks_current_profile() {
        let (app, _dir, _claude_path) = test_app();
        app.save_profile("work").unwrap();
        app.profiles
            .save("other", &account("other", "org"))
            .unwrap();

        let entries = app.list_profiles().unwrap();

        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .any(|entry| entry.name == "work" && entry.current));
        assert!(entries
            .iter()
            .any(|entry| entry.name == "other" && !entry.current));
    }

    #[test]
    fn current_profile_reports_saved_name() {
        let (app, _dir, _claude_path) = test_app();
        app.save_profile("work").unwrap();

        let current = app.current_profile().unwrap();

        assert_eq!(current.name, Some("work".to_string()));
        assert_eq!(current.oauth_account["accountUuid"], "old");
    }

    #[test]
    fn use_profile_switches_oauth_account_and_active_secret() {
        let (app, _dir, claude_path) = test_app();
        app.profiles
            .save("target", &account("new", "org2"))
            .unwrap();
        app.vault
            .store("target", &Secret::new(b"new-token".to_vec()))
            .unwrap();

        let result = app.use_profile("target").unwrap();

        assert_eq!(result.name, "target");
        assert_eq!(app.active.read().unwrap().expose(), b"new-token");
        let root: Value = serde_json::from_slice(&fs::read(claude_path).unwrap()).unwrap();
        assert_eq!(root["oauthAccount"]["accountUuid"], "new");
        assert_eq!(root["projects"]["keep"], true);
    }

    #[test]
    fn use_profile_rolls_back_when_active_write_fails() {
        let (app, _dir, claude_path) = test_app();
        app.profiles
            .save("target", &account("new", "org2"))
            .unwrap();
        app.vault
            .store("target", &Secret::new(b"new-token".to_vec()))
            .unwrap();
        app.active.fail_next_write();

        let err = app.use_profile("target").unwrap_err();

        assert!(format!("{err:#}").contains("rolled back"));
        assert_eq!(app.active.read().unwrap().expose(), b"old-token");
        let root: Value = serde_json::from_slice(&fs::read(claude_path).unwrap()).unwrap();
        assert_eq!(root["oauthAccount"]["accountUuid"], "old");
        assert_eq!(root["projects"]["keep"], true);
    }

    #[test]
    fn pre_use_hook_failure_does_not_switch_account() {
        let hooks = RecordingHooks::default();
        hooks.fail_for(HookPhase::PreUse, "target");
        let (app, _dir, claude_path) = test_app_with_hooks(hooks.clone());
        app.profiles
            .save("target", &account("new", "org2"))
            .unwrap();
        app.vault
            .store("target", &Secret::new(b"new-token".to_vec()))
            .unwrap();

        let err = app.use_profile("target").unwrap_err();

        assert!(format!("{err:#}").contains("pre-use hook failed"));
        assert_eq!(
            hooks.calls(),
            vec![(HookPhase::PreUse, "target".to_string())]
        );
        assert_eq!(app.active.read().unwrap().expose(), b"old-token");
        let root: Value = serde_json::from_slice(&fs::read(claude_path).unwrap()).unwrap();
        assert_eq!(root["oauthAccount"]["accountUuid"], "old");
    }

    #[test]
    fn post_use_hook_failure_rolls_back_and_runs_previous_post_hook() {
        let hooks = RecordingHooks::default();
        hooks.fail_for(HookPhase::PostUse, "target");
        let (app, _dir, claude_path) = test_app_with_hooks(hooks.clone());
        app.save_profile("work").unwrap();
        app.profiles
            .save("target", &account("new", "org2"))
            .unwrap();
        app.vault
            .store("target", &Secret::new(b"new-token".to_vec()))
            .unwrap();

        let err = app.use_profile("target").unwrap_err();

        assert!(format!("{err:#}").contains("rolled back"));
        assert_eq!(
            hooks.calls(),
            vec![
                (HookPhase::PreUse, "target".to_string()),
                (HookPhase::PostUse, "target".to_string()),
                (HookPhase::PostUse, "work".to_string()),
            ]
        );
        assert_eq!(app.active.read().unwrap().expose(), b"old-token");
        let root: Value = serde_json::from_slice(&fs::read(claude_path).unwrap()).unwrap();
        assert_eq!(root["oauthAccount"]["accountUuid"], "old");
    }

    #[test]
    fn use_profile_snapshots_previous_account_and_token() {
        let (app, _dir, _claude_path) = test_app();
        app.profiles
            .save("target", &account("new", "org2"))
            .unwrap();
        app.vault
            .store("target", &Secret::new(b"new-token".to_vec()))
            .unwrap();

        app.use_profile("target").unwrap();

        assert_eq!(
            app.vault.load(RESERVED_PREVIOUS_NAME).unwrap().expose(),
            b"old-token"
        );
        assert_eq!(app.previous.load().unwrap()["accountUuid"], "old");
    }

    #[test]
    fn use_previous_returns_to_prior_account_and_toggles() {
        let (app, _dir, claude_path) = test_app();
        app.profiles
            .save("target", &account("new", "org2"))
            .unwrap();
        app.vault
            .store("target", &Secret::new(b"new-token".to_vec()))
            .unwrap();
        app.use_profile("target").unwrap();

        let back = app.use_previous().unwrap();

        assert_eq!(app.active.read().unwrap().expose(), b"old-token");
        let root: Value = serde_json::from_slice(&fs::read(&claude_path).unwrap()).unwrap();
        assert_eq!(root["oauthAccount"]["accountUuid"], "old");
        assert_eq!(back.name, "-", "old account is unsaved");

        let forward = app.use_previous().unwrap();
        assert_eq!(app.active.read().unwrap().expose(), b"new-token");
        assert_eq!(forward.name, "target", "new account is saved");
    }

    #[test]
    fn use_previous_without_history_errors() {
        let (app, _dir, _claude_path) = test_app();
        assert!(app.use_previous().is_err());
    }

    #[test]
    fn remove_profile_deletes_metadata_and_secret() {
        let (app, _dir, _claude_path) = test_app();
        app.save_profile("work").unwrap();

        app.remove_profile("work").unwrap();

        assert!(app.profiles.load("work").is_err());
        assert!(app.vault.load("work").is_err());
    }

    #[test]
    fn format_identity_uses_email_and_org_without_extra_fields() {
        let label = format_identity(&account("old", "org"));
        assert_eq!(label, "old@example.com / org");
    }
}

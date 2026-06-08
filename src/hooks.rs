use crate::fsutil;
use crate::profile::{identity_summary, validate_profile_name};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPhase {
    PreUse,
    PostUse,
}

impl HookPhase {
    pub fn config_key(self) -> &'static str {
        match self {
            Self::PreUse => "preUse",
            Self::PostUse => "postUse",
        }
    }

    pub fn env_value(self) -> &'static str {
        match self {
            Self::PreUse => "pre-use",
            Self::PostUse => "post-use",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PreUse => "pre",
            Self::PostUse => "post",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookSpec {
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl HookSpec {
    pub fn new(command: impl Into<String>, args: Vec<String>) -> Result<Self> {
        let spec = Self {
            command: command.into(),
            args,
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn from_argv(mut argv: Vec<String>) -> Result<Self> {
        if argv.is_empty() {
            bail!("hook command cannot be empty");
        }
        let command = argv.remove(0);
        Self::new(command, argv)
    }

    pub fn display_command(&self) -> String {
        std::iter::once(&self.command)
            .chain(self.args.iter())
            .map(|part| quote_for_display(part))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn validate(&self) -> Result<()> {
        if self.command.is_empty() {
            bail!("hook command cannot be empty");
        }
        if self.command.contains('\0') || self.args.iter().any(|arg| arg.contains('\0')) {
            bail!("hook command cannot contain NUL bytes");
        }
        Ok(())
    }

    fn run(&self, phase: HookPhase, context: &SwitchHookContext) -> Result<()> {
        let mut command = Command::new(&self.command);
        command.args(&self.args);
        set_hook_env(&mut command, phase, context);

        let status = command
            .status()
            .with_context(|| format!("spawn hook command '{}'", self.command))?;
        if !status.success() {
            bail!("hook command exited with {status}");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileHooks {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_use: Vec<HookSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_use: Vec<HookSpec>,
}

impl ProfileHooks {
    fn hooks(&self, phase: HookPhase) -> &[HookSpec] {
        match phase {
            HookPhase::PreUse => &self.pre_use,
            HookPhase::PostUse => &self.post_use,
        }
    }

    fn hooks_mut(&mut self, phase: HookPhase) -> &mut Vec<HookSpec> {
        match phase {
            HookPhase::PreUse => &mut self.pre_use,
            HookPhase::PostUse => &mut self.post_use,
        }
    }

    fn is_empty(&self) -> bool {
        self.pre_use.is_empty() && self.post_use.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HooksConfig {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, ProfileHooks>,
}

impl HooksConfig {
    pub fn validate(&self) -> Result<()> {
        for (profile, hooks) in &self.profiles {
            validate_profile_name(profile)
                .with_context(|| format!("invalid hook profile '{profile}'"))?;
            for spec in hooks.pre_use.iter().chain(hooks.post_use.iter()) {
                spec.validate()
                    .with_context(|| format!("invalid hook for profile '{profile}'"))?;
            }
        }
        Ok(())
    }

    pub fn add(&mut self, profile: &str, phase: HookPhase, spec: HookSpec) -> Result<usize> {
        validate_profile_name(profile)?;
        spec.validate()?;

        let hooks = self.profiles.entry(profile.to_string()).or_default();
        let phase_hooks = hooks.hooks_mut(phase);
        phase_hooks.push(spec);
        Ok(phase_hooks.len())
    }

    pub fn remove(
        &mut self,
        profile: &str,
        phase: HookPhase,
        one_based_index: usize,
    ) -> Result<HookSpec> {
        validate_profile_name(profile)?;
        if one_based_index == 0 {
            bail!("hook index starts at 1");
        }

        let hooks = self
            .profiles
            .get_mut(profile)
            .with_context(|| format!("profile '{profile}' has no hooks"))?;
        let phase_hooks = hooks.hooks_mut(phase);
        if one_based_index > phase_hooks.len() {
            bail!(
                "profile '{profile}' has no {} hook #{one_based_index}",
                phase.label()
            );
        }

        let removed = phase_hooks.remove(one_based_index - 1);
        if hooks.is_empty() {
            self.profiles.remove(profile);
        }
        Ok(removed)
    }

    pub fn hooks_for(&self, profile: &str, phase: HookPhase) -> &[HookSpec] {
        self.profiles
            .get(profile)
            .map(|hooks| hooks.hooks(phase))
            .unwrap_or(&[])
    }

    pub fn entries(&self, profile_filter: Option<&str>) -> Result<Vec<HookEntry>> {
        if let Some(profile) = profile_filter {
            validate_profile_name(profile)?;
        }

        let mut entries = Vec::new();
        for (profile, hooks) in &self.profiles {
            if profile_filter.is_some_and(|filter| filter != profile) {
                continue;
            }
            for (index, spec) in hooks.pre_use.iter().enumerate() {
                entries.push(HookEntry::new(profile, HookPhase::PreUse, index + 1, spec));
            }
            for (index, spec) in hooks.post_use.iter().enumerate() {
                entries.push(HookEntry::new(profile, HookPhase::PostUse, index + 1, spec));
            }
        }
        Ok(entries)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookEntry {
    pub profile: String,
    pub phase: HookPhase,
    pub index: usize,
    pub spec: HookSpec,
}

impl HookEntry {
    fn new(profile: &str, phase: HookPhase, index: usize, spec: &HookSpec) -> Self {
        Self {
            profile: profile.to_string(),
            phase,
            index,
            spec: spec.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HookConfigStore {
    path: PathBuf,
}

impl HookConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<HooksConfig> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(HooksConfig::default());
            }
            Err(err) => return Err(err).with_context(|| format!("read {}", self.path.display())),
        };

        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(HooksConfig::default());
        }

        let config: HooksConfig = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse {}", self.path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, config: &HooksConfig) -> Result<()> {
        config.validate()?;
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fsutil::ensure_private_dir(parent)?;
        }
        let value = serde_json::to_value(config).context("serialize hook config")?;
        fsutil::write_json_atomic(&self.path, &value, Some(0o600))
            .with_context(|| format!("write {}", self.path.display()))
    }

    pub fn ensure_exists(&self) -> Result<()> {
        if self.path.exists() {
            return Ok(());
        }
        self.save(&HooksConfig::default())
    }

    pub fn add(&self, profile: &str, phase: HookPhase, spec: HookSpec) -> Result<usize> {
        let mut config = self.load()?;
        let index = config.add(profile, phase, spec)?;
        self.save(&config)?;
        Ok(index)
    }

    pub fn remove(
        &self,
        profile: &str,
        phase: HookPhase,
        one_based_index: usize,
    ) -> Result<HookSpec> {
        let mut config = self.load()?;
        let removed = config.remove(profile, phase, one_based_index)?;
        self.save(&config)?;
        Ok(removed)
    }
}

#[derive(Debug, Clone)]
pub struct SwitchHookContext {
    pub target_profile: String,
    pub target_account: Value,
    pub previous_profile: Option<String>,
    pub previous_account: Value,
    pub rollback: bool,
}

impl SwitchHookContext {
    pub fn new(
        target_profile: impl Into<String>,
        target_account: Value,
        previous_profile: Option<String>,
        previous_account: Value,
    ) -> Self {
        Self {
            target_profile: target_profile.into(),
            target_account,
            previous_profile,
            previous_account,
            rollback: false,
        }
    }

    pub fn previous_as_rollback_target(&self) -> Option<Self> {
        let target_profile = self.previous_profile.clone()?;
        Some(Self {
            target_profile,
            target_account: self.previous_account.clone(),
            previous_profile: Some(self.target_profile.clone()),
            previous_account: self.target_account.clone(),
            rollback: true,
        })
    }
}

pub trait SwitchHooks {
    fn run(&self, phase: HookPhase, context: &SwitchHookContext) -> Result<()>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopHooks;

impl SwitchHooks for NoopHooks {
    fn run(&self, _phase: HookPhase, _context: &SwitchHookContext) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ConfiguredHooks {
    store: HookConfigStore,
}

impl ConfiguredHooks {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            store: HookConfigStore::new(path),
        }
    }
}

impl SwitchHooks for ConfiguredHooks {
    fn run(&self, phase: HookPhase, context: &SwitchHookContext) -> Result<()> {
        let config = self.store.load()?;
        run_configured_hooks(&config, phase, context)
    }
}

pub fn run_configured_hooks(
    config: &HooksConfig,
    phase: HookPhase,
    context: &SwitchHookContext,
) -> Result<()> {
    for (index, spec) in config
        .hooks_for(&context.target_profile, phase)
        .iter()
        .enumerate()
    {
        spec.run(phase, context).with_context(|| {
            format!(
                "{} hook #{} for profile '{}' failed",
                phase.label(),
                index + 1,
                context.target_profile
            )
        })?;
    }
    Ok(())
}

fn set_hook_env(command: &mut Command, phase: HookPhase, context: &SwitchHookContext) {
    command.env("CCSWAP_HOOK_PHASE", phase.env_value());
    command.env(
        "CCSWAP_HOOK_ROLLBACK",
        if context.rollback { "1" } else { "0" },
    );
    command.env("CCSWAP_TARGET_PROFILE", &context.target_profile);
    command.env(
        "CCSWAP_PREVIOUS_PROFILE",
        context.previous_profile.as_deref().unwrap_or(""),
    );
    command.env(
        "CCSWAP_TARGET_IDENTITY",
        identity_summary(&context.target_account),
    );
    command.env(
        "CCSWAP_PREVIOUS_IDENTITY",
        identity_summary(&context.previous_account),
    );
    set_account_env(command, "TARGET", &context.target_account);
    set_account_env(command, "PREVIOUS", &context.previous_account);
}

fn set_account_env(command: &mut Command, prefix: &str, account: &Value) {
    for (field, suffix) in [
        ("accountUuid", "ACCOUNT_UUID"),
        ("organizationUuid", "ORGANIZATION_UUID"),
        ("emailAddress", "EMAIL"),
        ("organizationName", "ORGANIZATION_NAME"),
        ("displayName", "DISPLAY_NAME"),
    ] {
        if let Some(value) = account.get(field).and_then(Value::as_str) {
            command.env(format!("CCSWAP_{prefix}_{suffix}"), value);
        }
    }
}

fn quote_for_display(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'.' | b'_' | b'-' | b':' | b'=' | b'+'))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn config_adds_lists_and_removes_hooks() {
        let mut config = HooksConfig::default();

        let index = config
            .add(
                "max",
                HookPhase::PostUse,
                HookSpec::new("/bin/echo", vec!["max".to_string()]).unwrap(),
            )
            .unwrap();
        assert_eq!(index, 1);

        let entries = config.entries(None).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].profile, "max");
        assert_eq!(entries[0].phase, HookPhase::PostUse);
        assert_eq!(entries[0].index, 1);

        let removed = config.remove("max", HookPhase::PostUse, 1).unwrap();
        assert_eq!(removed.command, "/bin/echo");
        assert!(config.profiles.is_empty());
    }

    #[test]
    fn config_rejects_invalid_profile_names() {
        let mut config = HooksConfig::default();
        assert!(config
            .add(
                "../max",
                HookPhase::PostUse,
                HookSpec::new("/bin/echo", Vec::new()).unwrap(),
            )
            .is_err());
    }

    #[test]
    fn store_round_trips_json() {
        let dir = tempdir().unwrap();
        let store = HookConfigStore::new(dir.path().join("hooks.json"));

        let index = store
            .add(
                "max",
                HookPhase::PreUse,
                HookSpec::new("/bin/echo", vec!["hello".to_string()]).unwrap(),
            )
            .unwrap();
        assert_eq!(index, 1);

        let config = store.load().unwrap();
        assert_eq!(config.hooks_for("max", HookPhase::PreUse).len(), 1);
    }

    #[test]
    fn rollback_context_targets_previous_profile() {
        let context = SwitchHookContext::new(
            "max",
            json!({ "accountUuid": "max", "organizationUuid": "org" }),
            Some("offload".to_string()),
            json!({ "accountUuid": "offload", "organizationUuid": "org" }),
        );

        let rollback = context.previous_as_rollback_target().unwrap();

        assert_eq!(rollback.target_profile, "offload");
        assert_eq!(rollback.previous_profile, Some("max".to_string()));
        assert!(rollback.rollback);
        assert_eq!(rollback.target_account["accountUuid"], "offload");
    }
}

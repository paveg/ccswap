use anyhow::{Context, Result};
use etcetera::app_strategy::{AppStrategy, AppStrategyArgs, Xdg};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub home_dir: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub hooks_path: PathBuf,
    pub profiles_dir: PathBuf,
    pub file_vault_dir: PathBuf,
    pub previous_path: PathBuf,
    pub claude_json_path: PathBuf,
    pub linux_credentials_path: PathBuf,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let strategy = Xdg::new(AppStrategyArgs {
            top_level_domain: String::new(),
            author: String::new(),
            app_name: "ccswap".to_string(),
        })
        .context("resolve XDG directories")?;

        let state_dir = strategy
            .state_dir()
            .context("XDG strategy did not provide a state directory")?;

        Ok(Self::from_dirs(
            strategy.home_dir().to_path_buf(),
            strategy.config_dir(),
            strategy.data_dir(),
            state_dir,
        )
        .apply_overrides(
            env_path("CCSWAP_CLAUDE_JSON"),
            env_path("CCSWAP_CREDENTIALS_PATH"),
        ))
    }

    /// Replace the Claude file paths from explicit overrides (env vars). A
    /// `None` leaves the discovered default in place.
    pub fn apply_overrides(
        mut self,
        claude_json: Option<PathBuf>,
        credentials: Option<PathBuf>,
    ) -> Self {
        if let Some(path) = claude_json {
            self.claude_json_path = path;
        }
        if let Some(path) = credentials {
            self.linux_credentials_path = path;
        }
        self
    }

    pub fn from_dirs(
        home_dir: impl Into<PathBuf>,
        config_dir: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
    ) -> Self {
        let home_dir = home_dir.into();
        let config_dir = config_dir.into();
        let data_dir = data_dir.into();
        let state_dir = state_dir.into();

        Self {
            hooks_path: config_dir.join("hooks.json"),
            profiles_dir: data_dir.join("profiles"),
            file_vault_dir: data_dir.join("vault"),
            previous_path: state_dir.join("previous.json"),
            claude_json_path: home_dir.join(".claude.json"),
            linux_credentials_path: home_dir.join(".claude").join(".credentials.json"),
            home_dir,
            config_dir,
            data_dir,
            state_dir,
        }
    }

    pub fn with_claude_json(mut self, path: impl AsRef<Path>) -> Self {
        self.claude_json_path = path.as_ref().to_path_buf();
        self
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn apply_overrides_replaces_only_given_paths() {
        let base = Paths::from_dirs("/home/me", "/cfg/ccswap", "/data/ccswap", "/state/ccswap");

        let unchanged = base.clone().apply_overrides(None, None);
        assert_eq!(unchanged, base);

        let overridden = base.clone().apply_overrides(
            Some("/custom/claude.json".into()),
            Some("/custom/creds".into()),
        );
        assert_eq!(
            overridden.claude_json_path,
            Path::new("/custom/claude.json")
        );
        assert_eq!(
            overridden.linux_credentials_path,
            Path::new("/custom/creds")
        );
        assert_eq!(overridden.profiles_dir, base.profiles_dir);
    }

    #[test]
    fn builds_expected_xdg_layout() {
        let paths = Paths::from_dirs("/home/me", "/cfg/ccswap", "/data/ccswap", "/state/ccswap");

        assert_eq!(paths.claude_json_path, Path::new("/home/me/.claude.json"));
        assert_eq!(
            paths.linux_credentials_path,
            Path::new("/home/me/.claude/.credentials.json")
        );
        assert_eq!(paths.profiles_dir, Path::new("/data/ccswap/profiles"));
        assert_eq!(paths.hooks_path, Path::new("/cfg/ccswap/hooks.json"));
        assert_eq!(paths.file_vault_dir, Path::new("/data/ccswap/vault"));
        assert_eq!(
            paths.previous_path,
            Path::new("/state/ccswap/previous.json")
        );
    }
}

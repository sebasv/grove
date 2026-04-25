use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub repos: Vec<RepoConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub struct ThemeConfig {
    #[serde(default)]
    pub base: crate::theme::ThemeName,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct General {
    #[serde(default = "default_base_branch")]
    pub default_base_branch: String,
    /// When set, new worktrees are placed at `<worktree_root>/<repo>/<branch>`.
    /// When absent, worktrees are placed next to the repo (sibling strategy).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_root: Option<PathBuf>,
    /// Seconds between background `git fetch` ticks per repo.  Set to 0
    /// to disable the scheduler entirely (manual `r` still works).
    #[serde(default = "default_fetch_cadence_secs")]
    pub fetch_cadence_secs: u64,
}

impl Default for General {
    fn default() -> Self {
        Self {
            default_base_branch: default_base_branch(),
            worktree_root: None,
            fetch_cadence_secs: default_fetch_cadence_secs(),
        }
    }
}

fn default_base_branch() -> String {
    "main".to_string()
}

fn default_fetch_cadence_secs() -> u64 {
    300
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RepoConfig {
    pub name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    /// Per-repo override for worktree placement; inherits `general.worktree_root` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_root: Option<PathBuf>,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading config at {}", path.display()))?;
        toml::from_str(&content).with_context(|| format!("parsing config at {}", path.display()))
    }

    /// Load the config, falling back to defaults when the file is
    /// missing OR fails to parse.  In the parse-error case the second
    /// tuple element carries a human-readable message; callers surface
    /// it (sidebar warning + log) so a typo in the TOML doesn't silently
    /// wipe the user's repo list.
    pub fn load_or_default_lossy(path: &Path) -> (Self, Option<String>) {
        if !path.exists() {
            return (Self::default(), None);
        }
        match Self::load(path) {
            Ok(cfg) => (cfg, None),
            Err(err) => (Self::default(), Some(format!("{err:#}"))),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let serialized = toml::to_string_pretty(self).context("serializing config")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, &serialized)
            .with_context(|| format!("writing temp config at {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
        Ok(())
    }

    pub fn has_repo_named(&self, name: &str) -> bool {
        self.repos.iter().any(|r| r.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let original = Config {
            general: General {
                default_base_branch: "main".to_string(),
                worktree_root: None,
                fetch_cadence_secs: default_fetch_cadence_secs(),
            },
            theme: ThemeConfig::default(),
            repos: vec![
                RepoConfig {
                    name: "grove".to_string(),
                    path: PathBuf::from("/Users/sebas/dev/grove"),
                    base_branch: None,
                    worktree_root: None,
                },
                RepoConfig {
                    name: "dotfiles".to_string(),
                    path: PathBuf::from("/Users/sebas/dotfiles"),
                    base_branch: Some("master".to_string()),
                    worktree_root: None,
                },
            ],
        };
        let serialized = toml::to_string(&original).unwrap();
        let parsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn load_or_default_lossy_returns_defaults_for_missing_file() {
        let dir = std::env::temp_dir().join(format!(
            "grove-cfg-missing-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("config.toml");
        let (cfg, err) = Config::load_or_default_lossy(&path);
        assert_eq!(cfg, Config::default());
        assert!(err.is_none());
    }

    #[test]
    fn load_or_default_lossy_falls_back_on_parse_error() {
        let dir = std::env::temp_dir().join(format!(
            "grove-cfg-bad-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        // Missing closing quote: malformed TOML.
        std::fs::write(&path, "[general\ndefault_base_branch = \"main\n").unwrap();
        let (cfg, err) = Config::load_or_default_lossy(&path);
        assert_eq!(cfg, Config::default());
        let msg = err.expect("expected parse error message");
        assert!(
            msg.contains("parsing config"),
            "expected message to mention parsing, got: {msg}"
        );
    }

    #[test]
    fn parses_with_general_section_omitted() {
        let minimal = r#"
            [[repos]]
            name = "a"
            path = "/a"
        "#;
        let parsed: Config = toml::from_str(minimal).unwrap();
        assert_eq!(parsed.general.default_base_branch, "main");
        assert_eq!(parsed.repos.len(), 1);
    }
}

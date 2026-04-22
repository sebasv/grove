use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const TEMPLATE: &str = r#"# grove config
# Add one [[repos]] block per git repository you want grove to manage.
# Worktrees inside each repo are discovered automatically.

[general]
default_base_branch = "main"

# [[repos]]
# name = "myproject"
# path = "/path/to/myproject"
# base_branch = "main"   # optional; overrides general.default_base_branch
"#;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub keys: crate::keymap::Keymap,
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
    /// When `true` and tmux is installed, embedded terminals run inside a
    /// persistent tmux session so shells survive across grove restarts.
    #[serde(default)]
    pub tmux_backing: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            default_base_branch: default_base_branch(),
            tmux_backing: false,
        }
    }
}

fn default_base_branch() -> String {
    "main".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RepoConfig {
    pub name: String,
    pub path: PathBuf,
    #[serde(default)]
    pub base_branch: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            theme: ThemeConfig::default(),
            keys: crate::keymap::Keymap::default(),
            repos: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("reading config at {}", path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("parsing config at {}", path.display()))
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::default())
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

    pub fn write_template(path: &Path) -> Result<()> {
        if path.exists() {
            anyhow::bail!(
                "refusing to overwrite existing config at {}",
                path.display()
            );
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(path, TEMPLATE)
            .with_context(|| format!("writing template to {}", path.display()))?;
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
                tmux_backing: false,
            },
            theme: ThemeConfig::default(),
            keys: crate::keymap::Keymap::default(),
            repos: vec![
                RepoConfig {
                    name: "grove".to_string(),
                    path: PathBuf::from("/Users/sebas/dev/grove"),
                    base_branch: None,
                },
                RepoConfig {
                    name: "dotfiles".to_string(),
                    path: PathBuf::from("/Users/sebas/dotfiles"),
                    base_branch: Some("master".to_string()),
                },
            ],
        };
        let serialized = toml::to_string(&original).unwrap();
        let parsed: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(original, parsed);
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

    #[test]
    fn template_parses_as_valid_config() {
        let parsed: Config = toml::from_str(TEMPLATE).unwrap();
        assert!(parsed.repos.is_empty());
        assert_eq!(parsed.general.default_base_branch, "main");
    }
}

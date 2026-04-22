use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedState {
    pub schema_version: u32,
    #[serde(default)]
    pub ui: PersistedUi,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedUi {
    #[serde(default)]
    pub active_worktree: Option<ActiveWorktreeId>,
    #[serde(default)]
    pub expanded: HashMap<String, bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveWorktreeId {
    pub repo: String,
    pub branch: String,
}

pub fn load(path: &Path) -> Result<Option<PersistedState>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading state at {}", path.display()))?;
    let state: PersistedState = toml::from_str(&content)
        .with_context(|| format!("parsing state at {}", path.display()))?;
    if state.schema_version != CURRENT_SCHEMA_VERSION {
        eprintln!(
            "warning: ignoring state file at {} (schema_version={}, expected {})",
            path.display(),
            state.schema_version,
            CURRENT_SCHEMA_VERSION
        );
        return Ok(None);
    }
    Ok(Some(state))
}

pub fn save(state: &PersistedState, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let serialized = toml::to_string_pretty(state).context("serializing state")?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, serialized)
        .with_context(|| format!("writing temp state at {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let mut expanded = HashMap::new();
        expanded.insert("grove".to_string(), true);
        expanded.insert("dotfiles".to_string(), false);
        let original = PersistedState {
            schema_version: CURRENT_SCHEMA_VERSION,
            ui: PersistedUi {
                active_worktree: Some(ActiveWorktreeId {
                    repo: "grove".to_string(),
                    branch: "feat/sidebar".to_string(),
                }),
                expanded,
            },
        };
        let serialized = toml::to_string_pretty(&original).unwrap();
        let parsed: PersistedState = toml::from_str(&serialized).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn save_and_load_returns_same_state() {
        let dir = TempDir::new();
        let path = dir.join("state.toml");
        let state = PersistedState {
            schema_version: CURRENT_SCHEMA_VERSION,
            ui: PersistedUi::default(),
        };
        save(&state, &path).unwrap();
        let loaded = load(&path).unwrap().expect("should load");
        assert_eq!(state, loaded);
    }

    #[test]
    fn load_missing_file_returns_none() {
        let dir = TempDir::new();
        let path = dir.join("does-not-exist.toml");
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn load_rejects_incompatible_schema() {
        let dir = TempDir::new();
        let path = dir.join("state.toml");
        std::fs::write(&path, "schema_version = 99\n").unwrap();
        assert!(load(&path).unwrap().is_none());
    }

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "grove-test-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn join(&self, path: impl AsRef<std::path::Path>) -> std::path::PathBuf {
            self.0.join(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

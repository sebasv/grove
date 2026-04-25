use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// v1 keyed `active_worktree` by `(repo, branch)`. v2 keys it by absolute
// `path`, because a worktree's identity is its directory — branches change
// underneath (`git switch`) but paths don't. v1 state is dropped on first
// load; users lose the persisted active selection one time.
const CURRENT_SCHEMA_VERSION: u32 = 2;

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

/// Stable identity for a worktree across grove sessions. Path is the right
/// key because it survives branch switches inside the worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveWorktreeId {
    pub path: PathBuf,
}

pub fn load(path: &Path) -> Result<Option<PersistedState>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading state at {}", path.display()))?;

    // Peek at schema_version first. A schema bump renames/reshapes inner
    // fields (e.g. v1 → v2 changed `active_worktree` from `(repo, branch)`
    // to `path`), so a strict full parse would fail on an outdated file
    // before we ever check the version. Treat any older/unknown version
    // as "ignore quietly" rather than surfacing a TOML parse error.
    #[derive(Deserialize)]
    struct VersionPeek {
        #[serde(default)]
        schema_version: u32,
    }
    let peek: VersionPeek =
        toml::from_str(&content).with_context(|| format!("parsing state at {}", path.display()))?;
    if peek.schema_version != CURRENT_SCHEMA_VERSION {
        eprintln!(
            "warning: ignoring state file at {} (schema_version={}, expected {})",
            path.display(),
            peek.schema_version,
            CURRENT_SCHEMA_VERSION
        );
        return Ok(None);
    }

    let state: PersistedState =
        toml::from_str(&content).with_context(|| format!("parsing state at {}", path.display()))?;
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

pub fn current_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
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
                    path: PathBuf::from("/home/u/dev/grove-feat-sidebar"),
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
        let dir = tempdir();
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
        let dir = tempdir();
        let path = dir.join("does-not-exist.toml");
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn load_rejects_incompatible_schema() {
        let dir = tempdir();
        let path = dir.join("state.toml");
        std::fs::write(&path, "schema_version = 99\n").unwrap();
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn load_ignores_v1_state_with_old_active_worktree_shape() {
        // v1 keyed `active_worktree` by `(repo, branch)`. After bumping to
        // v2 the inner shape no longer parses, but `load` must still treat
        // the file as "outdated, ignore" rather than surfacing a TOML
        // parse error to the user.
        let dir = tempdir();
        let path = dir.join("state.toml");
        std::fs::write(
            &path,
            "schema_version = 1\n\
             [ui.active_worktree]\n\
             repo = \"grove\"\n\
             branch = \"feat/sidebar\"\n",
        )
        .unwrap();
        assert!(load(&path).unwrap().is_none());
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "grove-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

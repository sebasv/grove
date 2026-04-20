use std::path::PathBuf;

use anyhow::{anyhow, Result};
use directories::ProjectDirs;

#[allow(dead_code)] // fields consumed starting in PR 2 (config loading)
pub struct AppPaths {
    pub config_file: PathBuf,
    pub state_file: PathBuf,
    pub log_file: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Result<Self> {
        let dirs = ProjectDirs::from("", "", "grove")
            .ok_or_else(|| anyhow!("could not determine home directory for app data"))?;

        Ok(Self {
            config_file: dirs.config_dir().join("config.toml"),
            state_file: dirs.data_dir().join("state.toml"),
            log_file: dirs.cache_dir().join("grove.log"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_paths_on_this_platform() {
        let paths = AppPaths::resolve().expect("should resolve on any supported OS");
        assert!(paths.config_file.ends_with("config.toml"));
        assert!(paths.state_file.ends_with("state.toml"));
        assert!(paths.log_file.ends_with("grove.log"));
    }
}

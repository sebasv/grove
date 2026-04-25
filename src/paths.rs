use std::io::Write;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use directories::ProjectDirs;

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

/// Append `msg` to the user's grove log file.  Used for non-fatal warnings
/// from inside the TUI: writing to stderr would land on the alt screen and
/// corrupt the rendered UI.  All errors are swallowed — logging is best-effort.
pub fn log_warning(msg: &str) {
    let Ok(paths) = AppPaths::resolve() else {
        return;
    };
    if let Some(parent) = paths.log_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log_file)
    {
        let _ = writeln!(f, "{msg}");
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

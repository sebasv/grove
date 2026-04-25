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

/// Read the tail of the log file, returning at most the last `max_bytes`
/// of content split into lines.  A bounded read keeps the in-TUI viewer
/// responsive even when the log has grown to many MB.
///
/// Returns an empty vec if the log doesn't exist yet.
pub fn read_log_tail(max_bytes: u64) -> std::io::Result<Vec<String>> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(paths) = AppPaths::resolve() else {
        return Ok(Vec::new());
    };
    let mut file = match std::fs::File::open(&paths.log_file) {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let len = file.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    file.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    // Drop the first line if we didn't start from byte 0 — it's likely
    // truncated mid-message.
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    Ok(lines)
}

pub fn log_path() -> Option<PathBuf> {
    AppPaths::resolve().ok().map(|p| p.log_file)
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

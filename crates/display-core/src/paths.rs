//! XDG-compliant application paths.

use std::path::PathBuf;

pub const APP_DIR_NAME: &str = "monitor-layout";

/// `$MONITOR_LAYOUT_CONFIG_DIR` override (used by tests), else
/// `$XDG_CONFIG_HOME/monitor-layout`, else `~/.config/monitor-layout`.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MONITOR_LAYOUT_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join(APP_DIR_NAME));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join(APP_DIR_NAME))
}

/// `$XDG_STATE_HOME/monitor-layout`, else `~/.local/state/monitor-layout`
/// (used for logs and the pending-revert marker).
pub fn state_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MONITOR_LAYOUT_STATE_DIR") {
        return Some(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir).join(APP_DIR_NAME));
    }
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join(APP_DIR_NAME)
    })
}

pub fn profiles_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("profiles.json"))
}

pub fn prefs_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("prefs.json"))
}

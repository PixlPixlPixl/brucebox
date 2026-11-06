//! Where plugins live on this machine, which ones the user switched off, and
//! which folders the host will actually load.
//!
//! This crate is the shared answer to "what is installed, and may it run?":
//!
//! - [`data_dir`] — where Brucebox keeps state on this platform;
//! - [`config`] — the user's on/off switches (`plugins.toml`);
//! - [`manager`] — the roots that are searched and the per-folder verdict.
//!
//! It holds no library handles and no GPUI types, so the host reads it at
//! startup and the developer tooling (`cargo xtask plugin ...`) reads and writes
//! the very same files with the very same parser. That is deliberate: tooling
//! and host must never disagree about whether a package is installed or enabled.
//!
//! See `docs/adr/0006-plugin-folders-and-developer-workflow.md`.

pub mod config;
pub mod manager;

use std::path::PathBuf;

/// Environment variable that relocates the Brucebox data directory.
pub const DATA_DIR_ENV: &str = "BRUCEBOX_DATA_DIR";

pub use config::{CONFIG_FILE_NAME, CONFIG_SCHEMA, ConfigError, PluginConfig};
pub use manager::{
    PLUGIN_ROOTS_ENV, PackageReport, PackageState, PluginRoot, RootKind, Scan, roots,
};

/// The Brucebox user data directory for this platform.
///
/// `BRUCEBOX_DATA_DIR` overrides it, which is what makes a test run or a headless
/// check independent of the real installation.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join("brucebox");
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(app_data) = std::env::var_os("APPDATA") {
            return PathBuf::from(app_data).join("brucebox");
        }
    }

    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir).join("brucebox");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("brucebox");
    }

    // No home directory at all: keep state beside the process rather than
    // failing to start. Nothing in this foundation depends on the location.
    PathBuf::from(".brucebox")
}

/// The directory packages are installed into.
pub fn user_plugin_dir() -> PathBuf {
    data_dir().join("plugins")
}

/// The config file, inside the data directory.
pub fn config_path() -> PathBuf {
    data_dir().join(CONFIG_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_install_locations_hang_off_one_data_directory() {
        let dir = data_dir();

        assert_eq!(user_plugin_dir(), dir.join("plugins"));
        assert_eq!(config_path(), dir.join(CONFIG_FILE_NAME));
    }
}

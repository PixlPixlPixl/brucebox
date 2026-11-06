//! Which plugins the user has switched off.
//!
//! The config is a small file in the user data directory (`plugins.toml`), not a
//! database and not part of the workspace document: it describes *this
//! installation*, so copying the workspace elsewhere does not carry
//! "Agent Chat is disabled" with it, and a plugin folder keeps working for
//! another user of the same machine.
//!
//! ```toml
//! schema = 1
//!
//! [plugins]
//! "brucebox.agent-chat" = false
//! ```
//!
//! Only ids that are switched off are written. A plugin that is not listed is
//! enabled, which is what makes installing a folder enough to activate it; and a
//! config that names a plugin that is no longer installed is harmless.
//!
//! The file is read at startup and written when the user toggles a plugin in the
//! Plugins panel or runs the developer tooling. A missing file is not an error —
//! a fresh installation has none — and a corrupt one is reported and treated as
//! empty rather than preventing the host from starting.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::data_dir;

/// The config file's name inside the user data directory.
pub const CONFIG_FILE_NAME: &str = "plugins.toml";

/// The config schema this build reads and writes.
pub const CONFIG_SCHEMA: u32 = 1;

/// The user's plugin configuration: which plugins are switched off.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginConfig {
    disabled: BTreeMap<String, ()>,
}

impl PluginConfig {
    /// The config file inside the user data directory.
    pub fn path() -> PathBuf {
        data_dir().join(CONFIG_FILE_NAME)
    }

    /// Parse config text.
    pub fn parse(raw: &str) -> Result<Self, ConfigError> {
        let file: ConfigFile =
            toml::from_str(raw).map_err(|error| ConfigError::Malformed(error.to_string()))?;
        if file.schema != CONFIG_SCHEMA {
            return Err(ConfigError::Schema {
                found: file.schema,
                supported: CONFIG_SCHEMA,
            });
        }
        let disabled = file
            .plugins
            .into_iter()
            .filter(|(_, enabled)| !enabled)
            .map(|(id, _)| (id, ()))
            .collect();
        Ok(Self { disabled })
    }

    /// Read the config at `path`.
    ///
    /// Returns the config plus, if the file exists but cannot be read, a
    /// diagnostic describing that. A corrupt file is *not* silently treated as
    /// "everything enabled": the caller reports it and the user can delete it.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(raw) => match Self::parse(&raw) {
                Ok(config) => (config, None),
                Err(error) => (
                    Self::default(),
                    Some(format!("{}: {error}", path.display())),
                ),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(error) => (
                Self::default(),
                Some(format!("cannot read {}: {error}", path.display())),
            ),
        }
    }

    /// Write the config to `path`, creating the directory if needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, self.to_toml())?;
        std::fs::rename(&temporary, path)
    }

    /// Whether a plugin is enabled. Everything not switched off is enabled.
    pub fn is_enabled(&self, plugin_id: &str) -> bool {
        !self.disabled.contains_key(plugin_id)
    }

    /// Switch a plugin on or off. Returns whether this changed anything.
    pub fn set_enabled(&mut self, plugin_id: &str, enabled: bool) -> bool {
        if enabled {
            self.disabled.remove(plugin_id).is_some()
        } else {
            self.disabled.insert(plugin_id.to_owned(), ()).is_none()
        }
    }

    /// The ids that are switched off, in stable order.
    pub fn disabled(&self) -> impl Iterator<Item = &str> {
        self.disabled.keys().map(String::as_str)
    }

    /// Whether any plugin is switched off.
    pub fn is_empty(&self) -> bool {
        self.disabled.is_empty()
    }

    /// The config as TOML, in the shape [`Self::parse`] reads.
    pub fn to_toml(&self) -> String {
        let mut file = ConfigFile {
            schema: CONFIG_SCHEMA,
            plugins: BTreeMap::new(),
        };
        for id in self.disabled() {
            file.plugins.insert(id.to_owned(), false);
        }
        toml::to_string_pretty(&file).unwrap_or_else(|_| format!("schema = {CONFIG_SCHEMA}\n"))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ConfigFile {
    schema: u32,
    #[serde(default)]
    plugins: BTreeMap<String, bool>,
}

/// Why a config file was refused.
#[derive(Debug)]
pub enum ConfigError {
    /// The text is not a config.
    Malformed(String),
    /// The config schema is not the one this build reads.
    Schema {
        /// Declared schema.
        found: u32,
        /// Supported schema.
        supported: u32,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(detail) => write!(formatter, "cannot read the plugin config: {detail}"),
            Self::Schema { found, supported } => write!(
                formatter,
                "plugin config schema {found} is not supported (this build reads schema {supported})"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_plugin_is_enabled_and_only_switched_off_ones_are_written() {
        let mut config = PluginConfig::default();
        assert!(config.is_enabled("brucebox.git"));

        assert!(config.set_enabled("brucebox.git", false));
        assert!(!config.is_enabled("brucebox.git"));
        assert!(
            !config.set_enabled("brucebox.git", false),
            "no second change"
        );

        let text = config.to_toml();
        assert!(text.contains("schema = 1"), "{text}");
        assert!(text.contains("\"brucebox.git\" = false"), "{text}");
        assert!(!text.contains("brucebox.agent-chat"), "{text}");

        let parsed = PluginConfig::parse(&text).expect("written config parses");
        assert_eq!(parsed, config);
    }

    #[test]
    fn switching_a_plugin_back_on_removes_it_from_the_file() {
        let mut config = PluginConfig::default();
        config.set_enabled("brucebox.git", false);
        assert!(config.set_enabled("brucebox.git", true));
        assert!(config.is_empty());
        assert!(!config.to_toml().contains("brucebox.git"));
    }

    #[test]
    fn an_unknown_schema_is_refused_rather_than_assumed() {
        match PluginConfig::parse("schema = 2\n") {
            Err(ConfigError::Schema { found: 2, .. }) => {}
            other => panic!("expected a schema refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_corrupt_file_is_reported_and_treated_as_empty() {
        let path = std::env::temp_dir().join(format!(
            "brucebox-plugins-config-{}.toml",
            std::process::id()
        ));
        std::fs::write(&path, "this is { not toml").unwrap();

        let (config, diagnostic) = PluginConfig::load(&path);

        assert!(config.is_empty(), "the host still starts");
        assert!(
            diagnostic.is_some_and(|d| d.contains("plugins-config")),
            "the diagnostic names the file"
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_config_round_trips_through_a_file() {
        let path = std::env::temp_dir().join(format!(
            "brucebox-plugins-round-trip-{}.toml",
            std::process::id()
        ));
        let mut config = PluginConfig::default();
        config.set_enabled("brucebox.agent-chat", false);

        config.save(&path).expect("the config is written");
        let (read, diagnostic) = PluginConfig::load(&path);

        assert!(diagnostic.is_none());
        assert_eq!(read, config);
        assert!(!read.is_enabled("brucebox.agent-chat"));
        std::fs::remove_file(path).ok();
    }
}

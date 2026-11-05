//! Stable identifiers shared by the workspace, the panel catalog, and plugins.
//!
//! Three kinds of identity are deliberately kept apart:
//!
//! - [`PluginId`] names the plugin that owns a contribution, namespaced as
//!   `<vendor>.<feature>` so contributions from independent plugins cannot
//!   collide.
//! - [`PanelTypeId`] names one *kind* of panel (`<vendor>.<feature>.<panel>`).
//!   It is what layouts and the dock persist.
//! - [`PanelInstanceId`] names one *live* panel. Two panels of the same type
//!   have the same type id and different instance ids.
//!
//! Type ids reach GPUI Base Dock through [`Panel::panel_name`], which is
//! `&'static str`. Plugin ids are only known at runtime, so ids are interned
//! once and leaked for the life of the process. The leak is bounded by the
//! number of registered contributions, which is what makes it acceptable.
//!
//! [`Panel::panel_name`]: gpui_kit::base::dock::Panel::panel_name

use std::borrow::Borrow;
use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Longest identifier accepted, in bytes.
const MAX_ID_LEN: usize = 128;

/// Why an identifier was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdError {
    value: String,
    reason: &'static str,
}

impl IdError {
    fn new(value: &str, reason: &'static str) -> Self {
        Self {
            value: value.to_owned(),
            reason,
        }
    }

    /// The rejected value.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The rule the value broke.
    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid identifier `{}`: {}", self.value, self.reason)
    }
}

impl std::error::Error for IdError {}

fn validate(raw: &str) -> Result<(), &'static str> {
    if raw.is_empty() {
        return Err("must not be empty");
    }
    if raw.len() > MAX_ID_LEN {
        return Err("is longer than 128 bytes");
    }
    for segment in raw.split('.') {
        if segment.is_empty() {
            return Err("segments must not be empty");
        }
        let mut chars = segment.chars();
        let first = chars.next().expect("segments are never empty");
        if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
            return Err("segments must start with a lowercase letter or a digit");
        }
        if !segment
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return Err("only lowercase ascii letters, digits, '-' and '_' are allowed");
        }
    }
    Ok(())
}

fn validate_namespaced(raw: &str) -> Result<(), &'static str> {
    validate(raw)?;
    // A dot cannot be empty or trailing here, so at least two segments means
    // at least one namespace separator.
    if !raw.contains('.') {
        return Err("must be namespaced, e.g. `vendor.feature`");
    }
    Ok(())
}

/// A namespaced plugin identifier such as `brucebox.agent-chat`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PluginId(String);

impl PluginId {
    /// Parse and validate a namespaced plugin id.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, IdError> {
        let raw = raw.as_ref();
        validate_namespaced(raw).map_err(|reason| IdError::new(raw, reason))?;
        Ok(Self(raw.to_owned()))
    }

    /// The id as written in manifests and layouts.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for PluginId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// The stable identity of one kind of panel, such as `brucebox.git.status`.
///
/// This is what a layout persists and what the dock's panel registry is keyed
/// by. It says nothing about how many live panels of this kind exist.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PanelTypeId(String);

impl PanelTypeId {
    /// Build a panel type id from its owning plugin and a panel segment.
    pub fn new(plugin: &PluginId, name: &str) -> Result<Self, IdError> {
        if name.contains('.') {
            return Err(IdError::new(
                name,
                "is a single segment: the plugin namespace is added for you",
            ));
        }
        validate(name).map_err(|reason| IdError::new(name, reason))?;
        Ok(Self(format!("{plugin}.{name}")))
    }

    /// Parse a persisted panel type id.
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, IdError> {
        let raw = raw.as_ref();
        validate_namespaced(raw).map_err(|reason| IdError::new(raw, reason))?;
        Ok(Self(raw.to_owned()))
    }

    /// The id as persisted.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The plugin that owns this panel type.
    pub fn plugin(&self) -> PluginId {
        let namespace = self
            .0
            .rsplit_once('.')
            .expect("a panel type id always has a namespace")
            .0;
        PluginId(namespace.to_owned())
    }

    /// The panel segment, without the plugin namespace.
    pub fn name(&self) -> &str {
        self.0
            .rsplit_once('.')
            .expect("a panel type id always has a namespace")
            .1
    }

    /// This id as the `&'static str` GPUI Base Dock requires.
    pub fn as_static_str(&self) -> &'static str {
        intern(&self.0)
    }
}

impl fmt::Display for PanelTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for PanelTypeId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for PanelTypeId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

static INTERNED: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();

/// Intern a string for the life of the process.
///
/// Panels must answer [`Panel::panel_name`](gpui_kit::base::dock::Panel::panel_name)
/// with a `&'static str`, but plugin ids only exist at runtime. Interning once
/// keeps the dock registry and the panel's own answer identical, including
/// pointer identity, so repeated lookups cannot drift.
pub fn intern(value: &str) -> &'static str {
    let interned = INTERNED.get_or_init(|| Mutex::new(HashSet::new()));
    let mut interned = interned
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = interned.get(value) {
        return existing;
    }
    let leaked: &'static str = Box::leak(value.to_owned().into_boxed_str());
    interned.insert(leaked);
    leaked
}

static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// The unique identity of one live panel.
///
/// Minted when a panel is created and kept for as long as that panel exists,
/// independently of GPUI entity ids, so it can be persisted and restored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PanelInstanceId(u64);

impl PanelInstanceId {
    /// Mint the next instance id.
    pub fn next() -> Self {
        Self(NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed))
    }

    /// The raw value, for persisted state.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for PanelInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A panel type plus the instance: everything that identifies one panel.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PanelIdentity {
    panel_type: PanelTypeId,
    instance: PanelInstanceId,
}

impl PanelIdentity {
    pub fn new(panel_type: PanelTypeId, instance: PanelInstanceId) -> Self {
        Self {
            panel_type,
            instance,
        }
    }

    pub fn panel_type(&self) -> &PanelTypeId {
        &self.panel_type
    }

    pub fn instance(&self) -> PanelInstanceId {
        self.instance
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_ids_must_be_namespaced_and_slug_shaped() {
        assert_eq!(
            PluginId::parse("brucebox.agent-chat").unwrap().as_str(),
            "brucebox.agent-chat"
        );
        assert!(PluginId::parse("brucebox").is_err());
        assert!(PluginId::parse("Brucebox.Git").is_err());
        assert!(PluginId::parse("brucebox..git").is_err());
        assert!(PluginId::parse(".git").is_err());
        assert!(PluginId::parse("brucebox.git.").is_err());
        assert!(PluginId::parse("brucebox.g it").is_err());
        assert!(PluginId::parse("").is_err());
    }

    #[test]
    fn panel_type_ids_are_namespaced_under_their_plugin() {
        let plugin = PluginId::parse("brucebox.git").unwrap();
        let panel = PanelTypeId::new(&plugin, "status").unwrap();

        assert_eq!(panel.as_str(), "brucebox.git.status");
        assert_eq!(panel.plugin(), plugin);
        assert_eq!(panel.name(), "status");
        assert_eq!(PanelTypeId::parse("brucebox.git.status").unwrap(), panel);
        assert_eq!(
            PanelTypeId::parse("brucebox.git.status")
                .unwrap()
                .plugin()
                .as_str(),
            "brucebox.git"
        );
    }

    #[test]
    fn a_panel_segment_cannot_smuggle_in_a_namespace() {
        let plugin = PluginId::parse("brucebox.git").unwrap();
        assert!(PanelTypeId::new(&plugin, "status.extra").is_err());
        assert!(PanelTypeId::new(&plugin, "").is_err());
    }

    #[test]
    fn interning_returns_one_string_per_value() {
        let first = intern("brucebox.example.panel");
        let second = intern("brucebox.example.panel");

        assert_eq!(first, second);
        assert!(std::ptr::eq(first.as_ptr(), second.as_ptr()));
    }

    #[test]
    fn instance_ids_are_unique_and_distinct_from_type_ids() {
        let first = PanelInstanceId::next();
        let second = PanelInstanceId::next();

        assert_ne!(first, second);
        assert_ne!(first.get(), second.get());
    }
}

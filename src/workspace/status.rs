//! What the host's composition runtime is doing, for the panels that show it.
//!
//! The composition runtime runs off the application thread, so it cannot update
//! a view directly (SPEC §6). It publishes lifecycle facts here instead; panels
//! that care observe this global and re-render when it changes. The type is
//! plain data — no Cordis, no entities — so it can be read in a test without a
//! window and written by the runtime without a window.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use gpui_kit::{App, Global};

/// How many status lines the host keeps.
pub const STATUS_NOTES: usize = 12;

/// Lifecycle phase of one composition plugin.
///
/// This mirrors Cordis's stable states as the *host* reports them; it is not a
/// projection of internal lifecycle machinery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PluginPhase {
    /// The plugin was submitted and has not settled yet.
    #[default]
    Starting,
    /// Apply succeeded and the plugin's effects are live.
    Active,
    /// A required service is missing; apply has not run.
    Pending,
    /// Apply failed; the plugin's own contributions were rolled back.
    Failed,
    /// The plugin was disposed and its contributions withdrawn.
    Stopped,
}

impl PluginPhase {
    /// A short label for a status view.
    pub fn label(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Active => "active",
            Self::Pending => "pending",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

impl fmt::Display for PluginPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

/// One plugin's line in the host status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStatus {
    plugin: String,
    phase: PluginPhase,
    detail: Option<String>,
}

impl PluginStatus {
    /// The plugin id this row describes.
    pub fn plugin(&self) -> &str {
        &self.plugin
    }

    /// The phase the host last reported.
    pub fn phase(&self) -> PluginPhase {
        self.phase
    }

    /// Why the plugin is in this phase, when the host knows.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

/// The host's live composition status: one row per plugin plus recent notes.
#[derive(Debug, Default, Clone)]
pub struct HostStatus {
    plugins: BTreeMap<String, PluginStatus>,
    notes: VecDeque<String>,
}

impl HostStatus {
    /// Record a plugin's phase, keeping any separate note the host was told.
    pub fn set_phase(
        &mut self,
        plugin: impl Into<String>,
        phase: PluginPhase,
        detail: Option<String>,
    ) {
        let plugin = plugin.into();
        self.plugins.insert(
            plugin.clone(),
            PluginStatus {
                plugin,
                phase,
                detail,
            },
        );
    }

    /// Append a status line, dropping the oldest when the view is full.
    pub fn note(&mut self, note: impl Into<String>) {
        if self.notes.len() == STATUS_NOTES {
            self.notes.pop_front();
        }
        self.notes.push_back(note.into());
    }

    /// One plugin's row, if the host has reported on it.
    pub fn plugin(&self, plugin: &str) -> Option<&PluginStatus> {
        self.plugins.get(plugin)
    }

    /// Every reported plugin, ordered by id so the view is stable.
    pub fn plugins(&self) -> impl Iterator<Item = &PluginStatus> {
        self.plugins.values()
    }

    /// Recent status lines, oldest first.
    pub fn notes(&self) -> impl Iterator<Item = &str> {
        self.notes.iter().map(String::as_str)
    }

    /// Whether nothing has been reported yet.
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty() && self.notes.is_empty()
    }
}

impl Global for HostStatus {}

/// Install an empty status global if the host has not done so yet.
pub fn init(cx: &mut App) {
    if cx.try_global::<HostStatus>().is_none() {
        cx.set_global(HostStatus::default());
    }
}

/// The host status. Panics if [`init`] has not run.
pub fn status(cx: &App) -> &HostStatus {
    cx.global::<HostStatus>()
}

/// The host status, if one has been installed.
pub fn try_status(cx: &App) -> Option<&HostStatus> {
    cx.try_global::<HostStatus>()
}

/// Change the host status and let its observers re-render.
pub fn update(cx: &mut App, update: impl FnOnce(&mut HostStatus)) {
    let mut status = cx.try_global::<HostStatus>().cloned().unwrap_or_default();
    update(&mut status);
    cx.set_global(status);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_have_stable_labels() {
        assert_eq!(PluginPhase::default(), PluginPhase::Starting);
        assert_eq!(PluginPhase::Pending.to_string(), "pending");
        assert_eq!(PluginPhase::Stopped.label(), "stopped");
    }

    #[test]
    fn a_phase_update_keeps_the_row_keyed_by_plugin() {
        let mut status = HostStatus::default();
        status.set_phase("brucebox.demo", PluginPhase::Active, None);
        status.set_phase(
            "brucebox.git",
            PluginPhase::Pending,
            Some("requires brucebox.workspace".into()),
        );
        status.set_phase("brucebox.demo", PluginPhase::Stopped, None);

        assert_eq!(status.plugins().count(), 2);
        assert_eq!(
            status.plugin("brucebox.demo").map(PluginStatus::phase),
            Some(PluginPhase::Stopped)
        );
        assert_eq!(
            status.plugin("brucebox.git").and_then(PluginStatus::detail),
            Some("requires brucebox.workspace")
        );
    }

    #[test]
    fn notes_are_bounded_so_a_chatty_plugin_cannot_grow_the_view() {
        let mut status = HostStatus::default();
        for index in 0..(STATUS_NOTES + 4) {
            status.note(format!("note {index}"));
        }

        let notes: Vec<&str> = status.notes().collect();
        assert_eq!(notes.len(), STATUS_NOTES);
        assert_eq!(notes.first().copied(), Some("note 4"));
        assert_eq!(
            notes.last().copied(),
            Some(format!("note {}", STATUS_NOTES + 3).as_str())
        );
    }
}

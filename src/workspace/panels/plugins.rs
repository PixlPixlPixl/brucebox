//! The Plugins panel: what is installed, what the host did with it, and the
//! switch for each one.
//!
//! This is the folder manager's face (PLAN step 6). It reads the same reports the
//! host acted on at startup — one row per plugin folder, including the ones that
//! were skipped — so the panel never has to guess why a plugin is missing: the
//! reason is the one the host used.
//!
//! Switching a plugin on or off writes the config and asks for a restart. The
//! library of a running plugin is never unloaded (ADR 0005), so "off" cannot mean
//! "unloaded now"; it means "not admitted next time".

use gpui_kit::base::dock::{Panel, PanelEvent};
use gpui_kit::*;

use crate::plugins::{self, PackageState};
use crate::workspace::ids::{PanelIdentity, intern};
use crate::workspace::status::HostStatus;

/// Shows every discovered plugin package and lets the user switch one off.
pub struct PluginsPanel {
    identity: PanelIdentity,
    focus_handle: FocusHandle,
    /// Kept alive so the panel refreshes when a plugin's phase changes.
    _status: Subscription,
}

impl PluginsPanel {
    /// Build the panel for one identity.
    pub fn new(identity: PanelIdentity, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            identity,
            focus_handle: cx.focus_handle(),
            // Enabling and disabling a plugin changes the host status, so that is
            // what this panel listens to as well as its own notifications.
            _status: cx.observe_global::<HostStatus>(|_, cx| cx.notify()),
        })
    }

    /// One package row: identity, where it came from, its state, and its switch.
    fn row(&self, row: &RowData, cx: &mut Context<Self>) -> Div {
        let plugin_id = row.plugin_id.clone();
        let enabled = row.enabled;
        let toggle = div()
            .id(SharedString::from(format!("toggle-{plugin_id}")))
            .cursor_pointer()
            .text_color(rgb(0x8fb8e0))
            .child(if enabled { "Disable" } else { "Enable" })
            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                // The switch is the config, not the running process: writing it
                // now is what makes the next start honour the choice.
                if let Err(error) = plugins::set_enabled(cx, &plugin_id, !enabled) {
                    crate::workspace::status::update(cx, |status| status.note(error));
                }
                cx.notify();
            }));

        let reason = row
            .reason
            .clone()
            .map(|reason| div().text_color(rgb(0x8b93a1)).child(reason));

        div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .child(div().flex_1().child(row.title.clone()))
                    .child(div().text_color(rgb(row.color)).child(row.state))
                    .child(toggle),
            )
            .child(div().text_color(rgb(0x6f7684)).child(row.detail.clone()))
            .children(reason)
            .children(
                row.pending
                    .map(|pending| div().text_color(rgb(0xe2c08d)).child(pending)),
            )
    }
}

/// Everything one row needs, copied out of the plugin set so the borrow of the
/// application can end before the row's click handler takes it mutably.
struct RowData {
    plugin_id: String,
    title: String,
    state: &'static str,
    color: u32,
    reason: Option<String>,
    detail: String,
    enabled: bool,
    /// Set when the config and what is loaded disagree: the change lands on the
    /// next start, because a loaded library is never unloaded.
    pending: Option<&'static str>,
}

impl Panel for PluginsPanel {
    fn panel_name(&self) -> &'static str {
        intern(self.identity.panel_type().as_str())
    }
}

impl EventEmitter<PanelEvent> for PluginsPanel {}

impl Focusable for PluginsPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PluginsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(set) = plugins::loaded(cx) else {
            return message("Plugins are not available in this build.");
        };
        let rows: Vec<RowData> = set
            .reports()
            .iter()
            .map(|report| {
                let (state, color) = match report.state() {
                    PackageState::Ready => ("ready", 0x7dd3a0),
                    PackageState::Disabled => ("disabled", 0xb4a0d0),
                    PackageState::MissingDependency { .. } | PackageState::Refused { .. } => {
                        ("unavailable", 0xe08d8d)
                    }
                };
                let enabled = set.config().is_enabled(report.plugin_id());
                let version = if report.version().is_empty() {
                    String::new()
                } else {
                    format!(" {}", report.version())
                };
                RowData {
                    plugin_id: report.plugin_id().to_owned(),
                    title: format!("{}{version}", report.name()),
                    state,
                    color,
                    reason: report.state().reason(),
                    detail: format!(
                        "{} · {} root · {}",
                        report.plugin_id(),
                        report.root().kind().label(),
                        report.folder().display()
                    ),
                    enabled,
                    pending: match (report.state().is_ready(), enabled) {
                        (true, false) => Some("switches off at the next start"),
                        (false, true) if *report.state() == PackageState::Disabled => {
                            Some("switches on at the next start")
                        }
                        _ => None,
                    },
                }
            })
            .collect();
        let config_path = set.config_path().display().to_string();
        let note = set.notes().first().cloned();

        let panel = div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(16.))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Plugins"))
            .child(
                div()
                    .text_color(rgb(0x8b93a1))
                    .child(format!("Config: {config_path}")),
            )
            .children(if rows.is_empty() {
                vec![message(
                    "No plugin folders found. Install one with `cargo xtask plugin install \
                     <folder>`, or point BRUCEBOX_PLUGIN_DIR at a package.",
                )]
            } else {
                rows.iter().map(|row| self.row(row, cx)).collect()
            });

        match note {
            Some(note) => panel
                .child(div().mt(px(8.)).child("Diagnostics"))
                .child(div().text_color(rgb(0x8b93a1)).child(note)),
            None => panel,
        }
    }
}

/// A neutral one-line message for the empty and unavailable cases.
fn message(text: &str) -> Div {
    div()
        .p(px(16.))
        .text_color(rgb(0x8b93a1))
        .child(text.to_owned())
}

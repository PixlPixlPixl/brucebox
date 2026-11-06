//! The panel catalog: every registered contribution, and a way to open one.
//!
//! This is the discovery surface for a workspace whose panels come from plugins
//! (SPEC §4 stage 8.2). It is a normal panel — closeable, movable, floatable — and
//! not a permanent dock slot; `Ctrl+Shift+P` reopens it, so closing it costs
//! nothing.
//!
//! It is host-side GPUI rather than a plugin panel, deliberately: clicking rows
//! means real pointer handling, and inside a plugin library GPUI's own listeners
//! cannot fire (ADR 0005). Opening a panel goes through
//! [`crate::workspace::open_panel`], the same path any future host surface would
//! use.

use gpui_kit::base::dock::{Panel, PanelEvent};
use gpui_kit::*;

use crate::workspace::catalog::{self, PanelMultiplicity};
use crate::workspace::ids::{PanelIdentity, intern};
use crate::workspace::status::HostStatus;

/// The panel type id of this panel.
pub const PANEL_NAME: &str = "brucebox.demo.panels";

/// Lists every panel type the loaded plugins registered.
pub struct LauncherPanel {
    identity: PanelIdentity,
    focus_handle: FocusHandle,
    /// The last refusal, shown so a failed open is not silent.
    last_error: Option<String>,
    /// Kept alive so the list refreshes when a plugin's phase changes.
    _status: Subscription,
}

impl LauncherPanel {
    /// Build the catalog panel.
    pub fn new(identity: PanelIdentity, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            identity,
            focus_handle: cx.focus_handle(),
            last_error: None,
            _status: cx.observe_global::<HostStatus>(|_, cx| cx.notify()),
        })
    }
}

impl Panel for LauncherPanel {
    fn panel_name(&self) -> &'static str {
        intern(self.identity.panel_type().as_str())
    }
}

impl EventEmitter<PanelEvent> for LauncherPanel {}

impl Focusable for LauncherPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LauncherPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let groups = plugin_groups(cx);
        let rows: Vec<AnyElement> = groups
            .into_iter()
            .flat_map(|group| self.group(group, cx))
            .collect();

        let empty = rows.is_empty().then(|| {
            div()
                .text_color(rgb(0x8b93a1))
                .child("No panel contributions are registered.")
        });

        // The refusal goes right under the heading, where a click that does
        // nothing is visible without scrolling to the bottom of a long list.
        let error = self.last_error.clone().map(|error| {
            div()
                .text_color(rgb(0xe08d8d))
                .child(format!("Cannot open: {error}"))
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(16.))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Panels"))
            .child(
                div()
                    .text_color(rgb(0x8b93a1))
                    .child("Every panel the loaded plugins registered. Click one to open it."),
            )
            .children(error)
            .child(
                div()
                    .id("panel-list")
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .overflow_y_scroll()
                    .children(rows)
                    .children(empty),
            )
            .child(
                div()
                    .mt(px(8.))
                    .text_color(rgb(0x6f7684))
                    .child("Ctrl+Shift+P opens this catalog again"),
            )
    }
}

impl LauncherPanel {
    /// One plugin's heading and its panel rows.
    fn group(&self, group: PluginGroup, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if group.panels.is_empty() {
            return Vec::new();
        }
        let mut rows = vec![
            div()
                .mt(px(4.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(group.name.clone())
                .into_any_element(),
            div()
                .text_color(rgb(0x6f7684))
                .child(group.plugin_id.clone())
                .into_any_element(),
        ];
        for panel in group.panels {
            rows.push(self.panel_row(panel, cx));
        }
        rows
    }

    /// One clickable panel row.
    fn panel_row(&self, panel: PanelRow, cx: &mut Context<Self>) -> AnyElement {
        let panel_type = panel.panel_type;
        let multiplicity = match panel.multiplicity {
            PanelMultiplicity::Single => "single",
            PanelMultiplicity::Multiple => "multiple",
        };
        let open = panel.open;

        div()
            .id(SharedString::from(format!("open-{}", panel_type.as_str())))
            .flex()
            .flex_row()
            .gap(px(8.))
            .px(px(4.))
            .cursor_pointer()
            .hover(|style| style.bg(rgb(0x222833)))
            .child(div().flex_1().child(panel.title))
            .child(
                div()
                    .text_color(rgb(0x6f7684))
                    .child(format!("{multiplicity} · {open} open")),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.last_error = crate::workspace::open_panel(window, cx, &panel_type).err();
                cx.notify();
            }))
            .into_any_element()
    }
}

/// One plugin's contributions, as the panel lists them.
struct PluginGroup {
    name: String,
    plugin_id: String,
    panels: Vec<PanelRow>,
}

/// One panel type, with how many instances are open.
struct PanelRow {
    title: String,
    panel_type: crate::workspace::ids::PanelTypeId,
    multiplicity: PanelMultiplicity,
    open: usize,
}

/// How many instances of each panel type are open in this window.
fn open_counts(cx: &App) -> std::collections::BTreeMap<String, usize> {
    let Some(workspace) = cx
        .try_global::<crate::workspace::ActiveWorkspace>()
        .map(|active| active.0.clone())
    else {
        return std::collections::BTreeMap::new();
    };
    workspace
        .read(cx)
        .open_instances()
        .iter()
        .map(|(panel_type, count)| (panel_type.as_str().to_owned(), *count))
        .collect()
}

/// Snapshot the catalog, grouped by plugin, in registration order.
fn plugin_groups(cx: &App) -> Vec<PluginGroup> {
    let Some(catalog) = catalog::try_catalog(cx) else {
        return Vec::new();
    };
    let counts = open_counts(cx);
    catalog
        .plugins()
        .map(|plugin| {
            let panels = catalog
                .panels()
                .filter(|descriptor| descriptor.plugin() == plugin.id())
                .map(|descriptor| PanelRow {
                    title: descriptor.title().to_string(),
                    panel_type: descriptor.panel_type().clone(),
                    multiplicity: descriptor.multiplicity(),
                    open: counts
                        .get(descriptor.panel_type().as_str())
                        .copied()
                        .unwrap_or_default(),
                })
                .collect();
            PluginGroup {
                name: plugin.display_name().to_string(),
                plugin_id: plugin.id().as_str().to_owned(),
                panels,
            }
        })
        .collect()
}

//! Temporary proving panels.
//!
//! These panels exist only to keep the docking smoke test alive while the
//! catalog and workspace are rebuilt (PLAN step 0/1). They are registered
//! through exactly the same declaration path as the real plugins, so replacing
//! them with `brucebox.agent-chat` and `brucebox.git` (PLAN steps 5-7) needs no
//! change to workspace or dock construction. They are not a plugin product and
//! carry no business logic — except [`ActivityPanel`], which shows the host's
//! own composition status.

use gpui_kit::base::dock::{Panel, PanelEvent};
use gpui_kit::*;

use crate::workspace::ids::{PanelIdentity, intern};
use crate::workspace::status::{self, HostStatus, PluginStatus};

pub struct DemoPanel {
    identity: PanelIdentity,
    title: SharedString,
    body: SharedString,
    focus_handle: FocusHandle,
}

impl DemoPanel {
    pub fn new(
        identity: PanelIdentity,
        title: impl Into<SharedString>,
        body: impl Into<SharedString>,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self {
            identity,
            title: title.into(),
            body: body.into(),
            focus_handle: cx.focus_handle(),
        })
    }

    /// The type and instance id this panel was created for.
    pub fn identity(&self) -> &PanelIdentity {
        &self.identity
    }
}

impl Panel for DemoPanel {
    fn panel_name(&self) -> &'static str {
        intern(self.identity.panel_type().as_str())
    }
}

impl EventEmitter<PanelEvent> for DemoPanel {}

impl Focusable for DemoPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DemoPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(16.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title.clone()),
            )
            .child(self.body.clone())
            .child(div().text_color(rgb(0x8b93a1)).child(format!(
                "{} · instance {}",
                self.identity.panel_type(),
                self.identity.instance()
            )))
    }
}

/// The activity panel: what the host's composition runtime is doing.
///
/// This is the UI end of the runtime bridge. The runtime runs off the
/// application thread and cannot touch a view (SPEC §6), so it publishes status
/// data; this panel observes that data and re-renders when it changes. It is the
/// visible proof that plugin lifecycle reaches the interface without any
/// background thread holding GPUI state.
pub struct ActivityPanel {
    identity: PanelIdentity,
    focus_handle: FocusHandle,
    /// Kept alive so the panel is notified when the host status changes.
    _status: Subscription,
}

impl ActivityPanel {
    pub fn new(identity: PanelIdentity, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            identity,
            focus_handle: cx.focus_handle(),
            _status: cx.observe_global::<HostStatus>(|_, cx| cx.notify()),
        })
    }

    /// The type and instance id this panel was created for.
    pub fn identity(&self) -> &PanelIdentity {
        &self.identity
    }
}

impl Panel for ActivityPanel {
    fn panel_name(&self) -> &'static str {
        intern(self.identity.panel_type().as_str())
    }
}

impl EventEmitter<PanelEvent> for ActivityPanel {}

impl Focusable for ActivityPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// One plugin row: what it is, what phase it is in, and why.
fn plugin_row(plugin: &PluginStatus) -> Div {
    let phase = plugin.phase();
    let color = match phase {
        status::PluginPhase::Active => rgb(0x7dd3a0),
        status::PluginPhase::Pending => rgb(0xe2c08d),
        status::PluginPhase::Failed => rgb(0xe08d8d),
        status::PluginPhase::Starting | status::PluginPhase::Stopped => rgb(0x8b93a1),
    };

    let row = div()
        .flex()
        .flex_row()
        .gap(px(8.))
        .child(div().flex_1().child(plugin.plugin().to_owned()))
        .child(div().text_color(color).child(phase.label()));

    match plugin.detail() {
        Some(detail) => row.child(div().text_color(rgb(0x8b93a1)).child(detail.to_owned())),
        None => row,
    }
}

impl Render for ActivityPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = status::try_status(cx);
        let plugins: Vec<Div> = status
            .map(|status| status.plugins().map(plugin_row).collect())
            .unwrap_or_default();
        let notes: Vec<Div> = status
            .map(|status| {
                status
                    .notes()
                    .map(|note| div().text_color(rgb(0x8b93a1)).child(note.to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let recent = (!notes.is_empty()).then(|| div().mt(px(8.)).child("Recent"));

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(16.))
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Activity"))
            .child(div().text_color(rgb(0x8b93a1)).child("Composition runtime"))
            .children(plugins)
            .children(recent)
            .children(notes)
    }
}

//! Temporary proving panels.
//!
//! These panels exist only to keep the docking smoke test alive while the
//! catalog and workspace are rebuilt (PLAN step 0/1). They are registered
//! through exactly the same contribution path as the real plugins, so replacing
//! them with `brucebox.agent-chat` and `brucebox.git` (PLAN steps 5-7) needs no
//! change to workspace or dock construction. They are not a plugin product and
//! carry no business logic.

use gpui_kit::base::dock::{Panel, PanelEvent};
use gpui_kit::*;

use crate::workspace::ids::{PanelIdentity, intern};

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

//! The placeholder the workspace shows where a panel cannot be built.
//!
//! Missing panels are a normal state, not a crash: a plugin may be absent,
//! disabled, incompatible, or simply not installed yet. The placeholder keeps
//! the persisted panel state so a later save does not erase the user's layout,
//! and draws enough for the user to see which contribution is missing.

use std::sync::Arc;

use gpui_kit::base::dock::{Panel, PanelEvent, PanelState, PanelView};
use gpui_kit::*;

use crate::workspace::ids::{PanelTypeId, intern};

pub struct MissingPanel {
    panel_name: SharedString,
    owner_hint: Option<SharedString>,
    state: Option<PanelState>,
    detail: SharedString,
    focus_handle: FocusHandle,
}

impl MissingPanel {
    /// Stand in for a panel that a loaded layout named.
    pub fn for_state(
        state: &PanelState,
        detail: impl Into<SharedString>,
        cx: &mut App,
    ) -> Entity<Self> {
        let panel_name = state.panel_name.clone();
        Self::build(panel_name, Some(state.clone()), detail, cx)
    }

    /// Stand in for a panel type a layout asked for but nothing provides.
    pub fn for_type(
        panel_type: &PanelTypeId,
        detail: impl Into<SharedString>,
        cx: &mut App,
    ) -> Entity<Self> {
        Self::build(panel_type.as_str().to_owned(), None, detail, cx)
    }

    /// Stand in for a named panel, as the dock's restore path finds it.
    pub fn for_state_view(
        state: &PanelState,
        detail: impl Into<SharedString>,
        cx: &mut App,
    ) -> Arc<dyn PanelView> {
        Arc::new(Self::for_state(state, detail, cx))
    }

    /// Stand in for a panel type, as the layout resolver needs it.
    pub fn for_type_view(
        panel_type: &PanelTypeId,
        detail: impl Into<SharedString>,
        cx: &mut App,
    ) -> Arc<dyn PanelView> {
        Arc::new(Self::for_type(panel_type, detail, cx))
    }

    fn build(
        panel_name: String,
        state: Option<PanelState>,
        detail: impl Into<SharedString>,
        cx: &mut App,
    ) -> Entity<Self> {
        let owner_hint = PanelTypeId::parse(&panel_name)
            .ok()
            .map(|panel_type| panel_type.plugin().to_string().into());
        cx.new(|cx| Self {
            panel_name: panel_name.into(),
            owner_hint,
            state,
            detail: detail.into(),
            focus_handle: cx.focus_handle(),
        })
    }

    /// The persisted name of the panel this stands in for.
    pub fn panel_name(&self) -> &SharedString {
        &self.panel_name
    }

    /// Why the panel is unavailable.
    pub fn detail(&self) -> &SharedString {
        &self.detail
    }
}

impl Panel for MissingPanel {
    fn panel_name(&self) -> &'static str {
        intern(&self.panel_name)
    }

    fn dump(&self, _: &App) -> PanelState {
        // Hand the load-time state back unchanged so a save after a load keeps
        // the panel that could not be built.
        self.state
            .clone()
            .unwrap_or_else(|| PanelState::new(self.panel_name.to_string()))
    }
}

impl EventEmitter<PanelEvent> for MissingPanel {}

impl Focusable for MissingPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MissingPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .p(px(16.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Panel unavailable"),
            )
            .child(
                div()
                    .text_color(rgb(0xb0b7c3))
                    .child(self.panel_name.clone()),
            );

        if let Some(owner) = self.owner_hint.clone() {
            body = body.child(
                div()
                    .text_color(rgb(0x8b93a1))
                    .child(format!("Provided by plugin `{owner}`")),
            );
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(body)
            .child(div().text_color(rgb(0x8b93a1)).child(self.detail.clone()))
    }
}

/// Convenience used by the dock's restore bridge in the catalog.
pub fn view_for_state(
    state: &PanelState,
    detail: impl Into<SharedString>,
    cx: &mut App,
) -> Arc<dyn PanelView> {
    MissingPanel::for_state_view(state, detail, cx)
}

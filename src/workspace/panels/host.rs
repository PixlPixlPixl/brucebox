//! The host's handle around a plugin-created panel.
//!
//! GPUI Base Dock works with [`PanelView`] handles and asks a panel for its
//! persisted name. The workspace additionally needs to know *which*
//! contribution a live panel came from and which instance it is, so every
//! panel is wrapped in a [`HostPanel`]: the plugin's own entity stays the thing
//! that renders, while the host adds the catalog identity, keeps the persisted
//! `panel_name` equal to the panel type id, and delegates lifecycle hooks back
//! to the plugin's panel.
//!
//! The wrapper is deliberately not a component library: a plugin panel is a
//! normal GPUI entity implementing [`Panel`] however it likes.

use std::sync::Arc;

use gpui_kit::base::dock::{Panel, PanelEvent, PanelState, PanelView, TabGroup};
use gpui_kit::*;

use crate::workspace::ids::{PanelIdentity, PanelInstanceId, PanelTypeId};

pub struct HostPanel {
    identity: PanelIdentity,
    panel_name: &'static str,
    focus_handle: FocusHandle,
    inner: Arc<dyn PanelView>,
}

impl HostPanel {
    /// Wrap a panel created by a contribution.
    pub fn new(identity: PanelIdentity, inner: Arc<dyn PanelView>, cx: &mut App) -> Entity<Self> {
        let focus_handle = inner.focus_handle(cx);
        let panel_name = identity.panel_type().as_static_str();
        cx.new(|_| Self {
            identity,
            panel_name,
            focus_handle,
            inner,
        })
    }

    /// Wrap a panel and hand it straight to the dock.
    pub fn view(
        identity: PanelIdentity,
        inner: Arc<dyn PanelView>,
        cx: &mut App,
    ) -> Arc<dyn PanelView> {
        Arc::new(Self::new(identity, inner, cx))
    }

    /// The type and instance id of the wrapped panel.
    pub fn identity(&self) -> &PanelIdentity {
        &self.identity
    }

    /// The instance id of the wrapped panel.
    pub fn instance(&self) -> PanelInstanceId {
        self.identity.instance()
    }

    /// The panel type of the wrapped panel.
    pub fn panel_type(&self) -> &PanelTypeId {
        self.identity.panel_type()
    }

    /// The plugin's own panel entity.
    pub fn inner(&self) -> &Arc<dyn PanelView> {
        &self.inner
    }
}

impl Panel for HostPanel {
    fn panel_name(&self) -> &'static str {
        self.panel_name
    }

    fn visible(&self, cx: &App) -> bool {
        self.inner.visible(cx)
    }

    fn closable(&self, cx: &App) -> bool {
        self.inner.closable(cx)
    }

    fn zoomable(&self, cx: &App) -> bool {
        self.inner.zoomable(cx)
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.inner.set_active(active, window, cx);
    }

    fn set_zoomed(&mut self, zoomed: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.inner.set_zoomed(zoomed, window, cx);
    }

    fn on_added_to(
        &mut self,
        group: WeakEntity<TabGroup>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inner.on_added_to(group, window, cx);
    }

    fn on_removed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.inner.on_removed(window, cx);
    }

    fn dump(&self, cx: &App) -> PanelState {
        let mut state = self.inner.dump(cx);
        // The dock persists `panel_name`, and restore looks the panel up by
        // it. The host owns that name so a plugin cannot accidentally persist
        // a name the catalog does not know.
        state.panel_name = self.panel_name.to_owned();
        state
    }
}

impl EventEmitter<PanelEvent> for HostPanel {}

impl Focusable for HostPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HostPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.inner.view()
    }
}

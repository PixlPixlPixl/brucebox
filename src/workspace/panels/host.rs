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
//!
//! # Persisted identity
//!
//! GPUI Base Dock persists one [`PanelState`] per leaf and hands it back on
//! restore. A plugin panel's own `dump` only describes the plugin's state, so
//! the host would lose the panel *instance* across a restart. [`HostPanel`]
//! therefore writes its state inside a small envelope that records the scheme
//! version, the instance id, and the owning plugin, with the plugin's own info
//! nested unchanged. Restoring splits the envelope back apart before the plugin
//! factory is called.
//!
//! Keeping the envelope inside the leaf state rather than in a parallel table
//! means the dock tree stays the single source of truth for the layout: there is
//! no second list of panels to pair up with the tree after a restore, and an
//! unknown or disabled plugin keeps its envelope untouched because the
//! placeholder hands the whole leaf state back out.

use std::sync::Arc;

use gpui_kit::base::dock::{Panel, PanelEvent, PanelInfo, PanelState, PanelView, TabGroup};
use gpui_kit::*;
use serde::{Deserialize, Serialize};

use crate::workspace::ids::{PanelIdentity, PanelInstanceId, PanelTypeId};

/// Version of the host panel envelope written into `PanelState::info`.
const HOST_PANEL_STATE_SCHEMA: u32 = 1;

pub struct HostPanel {
    identity: PanelIdentity,
    panel_name: &'static str,
    focus_handle: FocusHandle,
    inner: Arc<dyn PanelView>,
    /// Set while the workspace is moving this panel to another dock area.
    transferring: bool,
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
            transferring: false,
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

    /// Record that the workspace is moving this panel, not closing it.
    ///
    /// The dock reports a panel that leaves its tree as removed, which is the
    /// signal a plugin tears down on. A transfer between the docked workspace
    /// and a floating surface keeps the same panel entity and state, so the
    /// notice is withheld until the panel lands; see [`Panel::on_removed`].
    pub fn begin_transfer(&mut self) {
        self.transferring = true;
    }

    /// Clear the transfer flag without a matching removal.
    pub fn end_transfer(&mut self) {
        self.transferring = false;
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
        // One flag, one removal: a transfer clears it here rather than on the
        // caller's happy path, so a move that fails half way cannot leave the
        // panel unable to hear a real close later.
        if self.transferring {
            self.transferring = false;
            return;
        }
        self.inner.on_removed(window, cx);
    }

    fn dump(&self, cx: &App) -> PanelState {
        // The dock persists `panel_name`, and restore looks the panel up by
        // it. The host owns that name so a plugin cannot accidentally persist
        // a name the catalog does not know.
        encode(&self.identity, self.panel_name, self.inner.dump(cx))
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

/// The envelope a host panel writes into its persisted leaf state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct HostPanelState {
    schema: u32,
    instance: u64,
    owner: String,
    /// The panel's own persisted info, carried through untouched.
    #[serde(default)]
    panel: Option<PanelInfo>,
    /// Container children the panel wrote, if any.
    #[serde(default)]
    children: Vec<PanelState>,
}

/// Wrap a panel's own state in the host envelope.
///
/// The leaf's `panel_name` is the panel type id, which is also the dock's
/// registry key, so it stays authoritative on the outside of the envelope.
pub(crate) fn encode(
    identity: &PanelIdentity,
    panel_name: &'static str,
    inner: PanelState,
) -> PanelState {
    let envelope = HostPanelState {
        schema: HOST_PANEL_STATE_SCHEMA,
        instance: identity.instance().get(),
        owner: identity.panel_type().plugin().to_string(),
        panel: Some(inner.info),
        children: inner.children,
    };

    let info = serde_json::to_value(&envelope)
        .map(PanelInfo::Panel)
        .unwrap_or(PanelInfo::Panel(serde_json::Value::Null));

    PanelState {
        panel_name: panel_name.to_owned(),
        children: Vec::new(),
        info,
    }
}

/// Recover the panel identity and the panel's own state from a persisted leaf.
///
/// A leaf without a host envelope — written by an older build, or by a layer
/// that only knows GPUI Base Dock — is restored with a fresh instance id, no
/// recorded owner, and is handed to the plugin unchanged.
pub(crate) fn restore_state(state: &PanelState) -> (PanelInstanceId, PanelState, Option<String>) {
    let Some(envelope) = decode(state) else {
        return (PanelInstanceId::next(), state.clone(), None);
    };

    let panel_state = PanelState {
        panel_name: state.panel_name.clone(),
        children: envelope.children,
        info: envelope
            .panel
            .unwrap_or(PanelInfo::Panel(serde_json::Value::Null)),
    };

    (
        PanelInstanceId::adopt(envelope.instance),
        panel_state,
        Some(envelope.owner),
    )
}

fn decode(state: &PanelState) -> Option<HostPanelState> {
    let PanelInfo::Panel(value) = &state.info else {
        return None;
    };
    let envelope: HostPanelState = serde_json::from_value(value.clone()).ok()?;
    (envelope.schema == HOST_PANEL_STATE_SCHEMA).then_some(envelope)
}

#[cfg(test)]
mod tests {
    // Explicit imports, not `use super::*`: the parent module glob-imports
    // `gpui_kit`, whose `test` macro would shadow the built-in `#[test]` here.
    use super::{HOST_PANEL_STATE_SCHEMA, decode, encode, restore_state};
    use crate::workspace::ids::{PanelIdentity, PanelInstanceId, PanelTypeId};
    use gpui_kit::base::dock::{PanelInfo, PanelState};

    fn identity() -> PanelIdentity {
        PanelIdentity::new(
            PanelTypeId::parse("brucebox.git.diff").unwrap(),
            PanelInstanceId::next(),
        )
    }

    fn panel_info() -> PanelInfo {
        PanelInfo::panel(serde_json::json!({"path": "src/main.rs"}))
    }

    #[test]
    fn the_envelope_round_trips_identity_and_panel_state() {
        let identity = identity();
        let inner = PanelState {
            panel_name: identity.panel_type().to_string(),
            children: Vec::new(),
            info: panel_info(),
        };

        let persisted = encode(&identity, "brucebox.git.diff", inner.clone());

        assert_eq!(persisted.panel_name, "brucebox.git.diff");
        let (instance, restored, owner) = restore_state(&persisted);
        assert_eq!(instance, identity.instance());
        assert_eq!(owner.as_deref(), Some("brucebox.git"));
        assert_eq!(restored.info, panel_info());
        assert_eq!(restored.panel_name, "brucebox.git.diff");
    }

    #[test]
    fn the_envelope_records_the_owning_plugin() {
        let identity = identity();
        let persisted = encode(
            &identity,
            "brucebox.git.diff",
            PanelState::new("brucebox.git.diff"),
        );
        let envelope = decode(&persisted).expect("the leaf carries a host envelope");

        assert_eq!(envelope.owner, "brucebox.git");
        assert_eq!(envelope.instance, identity.instance().get());
    }

    #[test]
    fn a_leaf_without_an_envelope_gets_a_fresh_instance() {
        let plain = PanelState {
            panel_name: "brucebox.git.status".to_owned(),
            children: Vec::new(),
            info: panel_info(),
        };

        let (first, restored, owner) = restore_state(&plain);
        let (second, _, _) = restore_state(&plain);

        assert_eq!(restored, plain);
        assert_eq!(owner, None, "a plain leaf records no owning plugin");
        assert_ne!(
            first, second,
            "a leaf with no persisted instance cannot share one"
        );
    }

    #[test]
    fn an_envelope_from_another_schema_is_ignored() {
        let mut persisted = encode(
            &identity(),
            "brucebox.git.diff",
            PanelState::new("brucebox.git.diff"),
        );
        persisted.info = PanelInfo::panel(serde_json::json!({
            "schema": HOST_PANEL_STATE_SCHEMA + 1,
            "instance": 7,
            "owner": "brucebox.git"
        }));

        let (instance, restored, owner) = restore_state(&persisted);

        assert_ne!(instance.get(), 7);
        assert_eq!(owner, None, "an unknown schema is not trusted");
        assert_eq!(restored.info, persisted.info);
    }

    #[test]
    fn an_adopted_instance_id_is_not_handed_out_again() {
        // A persisted id well above the current counter, without going near the
        // wrap point: other tests mint ids concurrently.
        let high = PanelInstanceId::next().get() + 1_000;
        let adopted = PanelInstanceId::adopt(high);
        let fresh = PanelInstanceId::next();

        assert_eq!(adopted.get(), high);
        assert!(fresh.get() > adopted.get());
    }
}

//! The panel contributions this build ships with.
//!
//! A contribution is two halves that never travel together:
//!
//! - a *declaration* — ids, titles, multiplicity — which the composition
//!   runtime carries and the workspace accepts
//!   ([`proving_declaration`]),
//! - a *factory* — code that builds a GPUI panel — which stays on the
//!   application thread ([`install_factories`]).
//!
//! Keeping them apart is what lets a plugin's apply body run off the
//! application thread (SPEC §6) while its panel is still a real GPUI entity.
//! [`catalog::register_declaration`] is the seam that pairs the two back
//! together.
//!
//! The panels here are the temporary proving panels. They are registered through
//! exactly the same declaration path the real plugins will use, so replacing
//! them with `brucebox.agent-chat` and `brucebox.git` (PLAN steps 5-7) needs no
//! change to the runtime, the catalog, or the workspace.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::dock::PanelView;
use gpui_kit::{App, Window};

use super::catalog::{
    self, CatalogError, PanelCreateError, PanelCreateRequest, PanelDescriptor, PanelFactory,
    PanelMultiplicity, PanelTypeId, PluginId, PluginRegistration,
};
use super::declaration::{PanelDeclaration, PluginDeclaration};
use super::panels::demo::{ActivityPanel, DemoPanel};
use super::panels::host::HostPanel;

/// Plugin id owning the temporary proving panels.
pub const PROVING_PLUGIN: &str = "brucebox.demo";

/// Display name of the proving plugin.
pub const PROVING_PLUGIN_NAME: &str = "Brucebox Proving Panels";

/// Panel type id of the welcome panel.
pub const WELCOME_PANEL: &str = "brucebox.demo.welcome";

/// Panel type id of the scratch pad panel (multi-instance).
pub const SCRATCH_PANEL: &str = "brucebox.demo.scratch";

/// Panel type id of the activity panel.
pub const ACTIVITY_PANEL: &str = "brucebox.demo.activity";

/// One built-in contribution: what it is called and how to build it.
struct Contribution {
    segment: &'static str,
    title: &'static str,
    multiplicity: PanelMultiplicity,
    factory: Rc<dyn PanelFactory>,
}

/// The contributions this build ships, in declaration order.
fn contributions() -> Vec<Contribution> {
    vec![
        Contribution {
            segment: "welcome",
            title: "Welcome",
            multiplicity: PanelMultiplicity::Single,
            factory: demo_factory("Welcome", "Welcome to Brucebox. This is your workspace."),
        },
        Contribution {
            segment: "scratch",
            title: "Scratch Pad",
            multiplicity: PanelMultiplicity::Multiple,
            factory: demo_factory(
                "Scratch Pad",
                "Drag this tab into another group to rearrange it.",
            ),
        },
        Contribution {
            segment: "activity",
            title: "Activity",
            multiplicity: PanelMultiplicity::Single,
            factory: activity_factory(),
        },
    ]
}

/// The proving plugin id. Panics only if a constant in this file is malformed.
fn proving_plugin_id() -> PluginId {
    PluginId::parse(PROVING_PLUGIN).expect("the built-in proving plugin id is a valid id")
}

/// The type id of one built-in contribution.
fn contribution_type(plugin: &PluginId, segment: &str) -> PanelTypeId {
    PanelTypeId::new(plugin, segment).expect("the built-in panel segments are valid ids")
}

/// Offer this build's panel factories to the application thread.
///
/// Must run before any declaration for these panels is accepted.
pub fn install_factories(cx: &mut App) {
    catalog::init_factories(cx);
    let plugin = proving_plugin_id();
    for contribution in contributions() {
        catalog::factories_mut(cx).insert(
            contribution_type(&plugin, contribution.segment),
            contribution.factory,
        );
    }
}

/// What the proving plugin contributes, as data the runtime can carry.
pub fn proving_declaration() -> PluginDeclaration {
    let plugin = proving_plugin_id();
    let mut declaration = PluginDeclaration::new(plugin.clone(), PROVING_PLUGIN_NAME);
    for contribution in contributions() {
        declaration = declaration.with_panel(
            PanelDeclaration::new(
                contribution_type(&plugin, contribution.segment),
                contribution.title,
            )
            .with_multiplicity(contribution.multiplicity),
        );
    }
    declaration
}

/// The temporary proving contributions as catalog descriptors.
///
/// Kept public so tests can check the ids the default preset names without
/// standing up a GPUI application.
pub fn proving_panels(plugin: &PluginId) -> Vec<PanelDescriptor> {
    contributions()
        .into_iter()
        .map(|contribution| {
            PanelDescriptor::new(
                plugin.clone(),
                contribution.segment,
                contribution.title,
                contribution.factory,
            )
            .expect("the proving panel segments are valid ids")
            .with_multiplicity(contribution.multiplicity)
        })
        .collect()
}

/// Register the proving plugin and its panels directly.
///
/// The composition runtime takes the declaration path instead; this is the same
/// registration without a runtime, which is what the tests that only care about
/// the catalog use.
pub fn register(cx: &mut App) -> Result<(), CatalogError> {
    install_factories(cx);
    let plugin = proving_plugin_id();
    catalog::catalog_mut(cx)
        .register_plugin(PluginRegistration::new(plugin.clone(), PROVING_PLUGIN_NAME))?;
    for descriptor in proving_panels(&plugin) {
        catalog::register_contribution(cx, descriptor)?;
    }
    Ok(())
}

/// A factory that builds one proving panel and hands it to the host.
fn demo_factory(title: &'static str, body: &'static str) -> Rc<dyn PanelFactory> {
    Rc::new(
        move |request: &PanelCreateRequest,
              _window: &mut Window,
              cx: &mut App|
              -> Result<Arc<dyn PanelView>, PanelCreateError> {
            let identity = request.identity().clone();
            let panel = DemoPanel::new(identity.clone(), title, body, cx);
            let panel: Arc<dyn PanelView> = Arc::new(panel);
            Ok(HostPanel::view(identity, panel, cx))
        },
    )
}

/// A factory that builds the activity panel, which shows the host status.
fn activity_factory() -> Rc<dyn PanelFactory> {
    Rc::new(
        |request: &PanelCreateRequest,
         _window: &mut Window,
         cx: &mut App|
         -> Result<Arc<dyn PanelView>, PanelCreateError> {
            let identity = request.identity().clone();
            let panel = ActivityPanel::new(identity.clone(), cx);
            let panel: Arc<dyn PanelView> = Arc::new(panel);
            Ok(HostPanel::view(identity, panel, cx))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_contributions_use_the_ids_the_default_preset_names() {
        let plugin = PluginId::parse(PROVING_PLUGIN).unwrap();
        let panels = proving_panels(&plugin);
        let ids: Vec<&str> = panels
            .iter()
            .map(|descriptor| descriptor.panel_type().as_str())
            .collect();

        assert_eq!(ids, vec![WELCOME_PANEL, SCRATCH_PANEL, ACTIVITY_PANEL]);
        assert!(
            panels
                .iter()
                .all(|descriptor| descriptor.plugin() == &plugin)
        );
    }

    #[test]
    fn the_welcome_and_activity_panels_are_single_instance() {
        let plugin = PluginId::parse(PROVING_PLUGIN).unwrap();
        let panels = proving_panels(&plugin);
        let multiplicity = |id: &str| {
            panels
                .iter()
                .find(|descriptor| descriptor.panel_type().as_str() == id)
                .expect("the panel is registered")
                .multiplicity()
        };

        assert_eq!(multiplicity(WELCOME_PANEL), PanelMultiplicity::Single);
        assert_eq!(multiplicity(ACTIVITY_PANEL), PanelMultiplicity::Single);
        assert_eq!(multiplicity(SCRATCH_PANEL), PanelMultiplicity::Multiple);
    }

    #[test]
    fn the_declaration_describes_the_same_contributions_as_the_descriptors() {
        let plugin = PluginId::parse(PROVING_PLUGIN).unwrap();
        let declaration = proving_declaration();
        let descriptors = proving_panels(&plugin);

        assert_eq!(declaration.plugin(), &plugin);
        assert_eq!(declaration.display_name(), PROVING_PLUGIN_NAME);
        assert_eq!(declaration.panels().len(), descriptors.len());
        for (declared, descriptor) in declaration.panels().iter().zip(&descriptors) {
            assert_eq!(declared.panel_type(), descriptor.panel_type());
            assert_eq!(declared.title(), descriptor.title().as_ref());
            assert_eq!(declared.multiplicity(), descriptor.multiplicity());
        }
    }
}

//! Panel contributions available at startup.
//!
//! Every panel the workspace can show is registered here through the catalog,
//! including the temporary proving panels. Later milestones replace the proving
//! panels with independently built plugin packages; the registration path stays
//! the same, so workspace and dock construction does not change.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::dock::PanelView;
use gpui_kit::{App, Window};

use super::catalog::{
    self, CatalogError, PanelCreateError, PanelCreateRequest, PanelDescriptor, PanelFactory,
    PanelMultiplicity, PluginId, PluginRegistration,
};
use super::panels::demo::DemoPanel;
use super::panels::host::HostPanel;

/// Plugin id owning the temporary proving panels.
pub const PROVING_PLUGIN: &str = "brucebox.demo";

/// Panel type id of the welcome panel.
pub const WELCOME_PANEL: &str = "brucebox.demo.welcome";

/// Panel type id of the scratch pad panel (multi-instance).
pub const SCRATCH_PANEL: &str = "brucebox.demo.scratch";

/// Panel type id of the activity panel.
pub const ACTIVITY_PANEL: &str = "brucebox.demo.activity";

/// Register the contributions this build ships with.
pub fn register(cx: &mut App) -> Result<(), CatalogError> {
    let plugin =
        PluginId::parse(PROVING_PLUGIN).expect("the built-in proving plugin id is a valid id");

    catalog::catalog_mut(cx).register_plugin(PluginRegistration::new(
        plugin.clone(),
        "Brucebox Proving Panels",
    ))?;

    for descriptor in proving_panels(&plugin) {
        catalog::register_contribution(cx, descriptor)?;
    }

    Ok(())
}

/// The temporary proving contributions.
///
/// Kept public so tests can check the ids the default preset names without
/// standing up a GPUI application.
pub fn proving_panels(plugin: &PluginId) -> Vec<PanelDescriptor> {
    vec![
        PanelDescriptor::new(
            plugin.clone(),
            "welcome",
            "Welcome",
            demo_factory("Welcome", "Welcome to Brucebox. This is your workspace."),
        )
        .expect("the welcome panel segment is a valid id")
        .with_multiplicity(PanelMultiplicity::Single),
        PanelDescriptor::new(
            plugin.clone(),
            "scratch",
            "Scratch Pad",
            demo_factory(
                "Scratch Pad",
                "Drag this tab into another group to rearrange it.",
            ),
        )
        .expect("the scratch pad panel segment is a valid id")
        .with_multiplicity(PanelMultiplicity::Multiple),
        PanelDescriptor::new(
            plugin.clone(),
            "activity",
            "Activity",
            demo_factory("Activity", "This panel can be moved independently."),
        )
        .expect("the activity panel segment is a valid id")
        .with_multiplicity(PanelMultiplicity::Single),
    ]
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
}

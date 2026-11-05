//! First-run workspace presets.
//!
//! A preset is data: workspace id plus a layout of panel type ids. The host
//! never hardcodes a sidebar, a main region, or an inspector slot — the default
//! arrangement is just a layout the host happens to start with, and it is
//! replaceable without touching docking code.

use gpui_kit::px;

use super::contributions::{ACTIVITY_PANEL, SCRATCH_PANEL, WELCOME_PANEL};
use super::ids::PanelTypeId;
use super::layout::{LayoutNode, WorkspaceLayout};

/// Schema version of the workspace layout the host writes.
pub const WORKSPACE_SCHEMA_VERSION: usize = 1;

/// Id of the default workspace.
pub const DEFAULT_WORKSPACE_ID: &str = "brucebox.default";

/// The layout a fresh installation starts with.
///
/// Panels are named by contribution id, so this is the only place the default
/// arrangement mentions a panel at all. This is bootstrap data: once the real
/// Agent Chat and Git plugins load, the default workspace is assembled from
/// their contributions instead (PLAN step 8), and saved layouts take over from
/// there (PLAN step 2).
pub fn default_layout() -> WorkspaceLayout {
    WorkspaceLayout::new(DEFAULT_WORKSPACE_ID, default_root())
}

fn default_root() -> LayoutNode {
    LayoutNode::h_split()
        .child(
            LayoutNode::tabs().panel(panel(WELCOME_PANEL)),
            Some(px(320.)),
        )
        .child(
            LayoutNode::tabs()
                .panel(panel(SCRATCH_PANEL))
                .panel(panel(ACTIVITY_PANEL))
                .active_index(0),
            None,
        )
}

fn panel(id: &str) -> PanelTypeId {
    PanelTypeId::parse(id).expect("the shipped default preset names valid panel type ids")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::catalog::PluginId;
    use crate::workspace::catalog::{PanelCatalog, PluginRegistration};
    use crate::workspace::contributions::{PROVING_PLUGIN, proving_panels};
    use crate::workspace::layout::plan;

    #[test]
    fn the_default_preset_installs_cleanly_against_the_shipped_contributions() {
        let plugin = PluginId::parse(PROVING_PLUGIN).unwrap();
        let mut catalog = PanelCatalog::new();
        catalog
            .register_plugin(PluginRegistration::new(plugin.clone(), "Proving Panels"))
            .unwrap();
        for descriptor in proving_panels(&plugin) {
            catalog
                .register_panel(std::rc::Rc::new(descriptor))
                .unwrap();
        }

        let layout = default_layout();
        let plan = plan(&layout, &catalog);

        assert_eq!(layout.id(), DEFAULT_WORKSPACE_ID);
        assert!(
            plan.diagnostics().is_empty(),
            "the default preset only names registered panels: {:?}",
            plan.diagnostics()
        );
    }

    #[test]
    fn the_default_preset_places_the_three_proving_panels() {
        let layout = default_layout();
        let LayoutNode::Split { axis, children } = layout.root() else {
            panic!("the default preset is a split");
        };

        assert_eq!(*axis, super::super::layout::SplitAxis::Horizontal);
        assert_eq!(children.len(), 2);

        let mut panel_ids: Vec<&str> = Vec::new();
        for (child, _) in children {
            let LayoutNode::Tabs { panels, .. } = child else {
                panic!("each default split child is a tab group");
            };
            panel_ids.extend(panels.iter().map(|panel| panel.as_str()));
        }
        panel_ids.sort_unstable();
        assert_eq!(
            panel_ids,
            vec![ACTIVITY_PANEL, SCRATCH_PANEL, WELCOME_PANEL]
        );
    }
}

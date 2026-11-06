//! First-run workspace presets, composed from what the plugins registered.
//!
//! A preset is data: a workspace id plus a layout of *panel type ids*. The preset
//! names ids, never panel implementations — so a plugin that is missing simply
//! contributes nothing, and the arrangement is built out of whatever is present.
//!
//! The default arrangement is a composition rule, not a fixed layout:
//!
//! 1. panels named in [`PREFERRED`] come first, in that order — the first-party
//!    contributions (Agent Chat, Git) and then the host's own panels;
//! 2. every other registered panel type follows in registration order, so a
//!    plugin nobody anticipated is still reachable;
//! 3. the first panel becomes the main column and the rest share a second column;
//!    an empty column is dropped.
//!
//! The result is replaceable in two senses: the user's saved document always
//! wins over it, and `Ctrl+Shift+R` rebuilds it from the *current* registrations
//! rather than from anything baked in at compile time.

use gpui_kit::px;
use gpui_kit::{App, SharedString};

use super::catalog::{self, PanelMultiplicity};
use super::ids::{PanelTypeId, PluginId};
use super::layout::{LayoutNode, WorkspaceLayout};

/// Schema version of the workspace layout the host writes.
pub const WORKSPACE_SCHEMA_VERSION: usize = 1;

/// Id of the default workspace.
pub const DEFAULT_WORKSPACE_ID: &str = "brucebox.default";

/// The width of the main column in the first-run preset.
const MAIN_COLUMN: f32 = 360.;

/// One registered panel type, as a preset sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogPanel {
    /// The plugin that registered it.
    pub plugin: PluginId,
    /// The plugin's display name.
    pub plugin_name: SharedString,
    /// The panel type id.
    pub panel_type: PanelTypeId,
    /// The default tab title.
    pub title: SharedString,
    /// How many instances the type allows.
    pub multiplicity: PanelMultiplicity,
}

/// The panel type ids the first-run preset tries to place first, in order.
///
/// These are *ids*, not implementations: when one is not registered (its plugin
/// is not installed), the preset moves on. They are also only a preference — the
/// rest of the catalog still gets placed.
pub const PREFERRED: &[&str] = &[
    "brucebox.agent-chat.chat",
    "brucebox.git.repository",
    "brucebox.git.diff",
    "brucebox.demo.welcome",
    "brucebox.demo.activity",
    "brucebox.demo.panels",
    "brucebox.demo.plugins",
    "brucebox.demo.scratch",
];

/// The first-run layout for the default workspace, from the live catalog.
pub fn default_layout(cx: &App) -> WorkspaceLayout {
    default_layout_from(&registered(cx))
}

/// A snapshot of the catalog, in registration order.
pub fn registered(cx: &App) -> Vec<CatalogPanel> {
    let Some(catalog) = catalog::try_catalog(cx) else {
        return Vec::new();
    };
    catalog
        .panels()
        .map(|descriptor| {
            let plugin = descriptor.plugin().clone();
            let plugin_name = catalog
                .plugins()
                .find(|registration| registration.id() == &plugin)
                .map(|registration| registration.display_name().clone())
                .unwrap_or_else(|| SharedString::from(plugin.as_str()));
            CatalogPanel {
                plugin,
                plugin_name,
                panel_type: descriptor.panel_type().clone(),
                title: descriptor.title().clone(),
                multiplicity: descriptor.multiplicity(),
            }
        })
        .collect()
}

/// Compose the first-run layout from a catalog snapshot.
///
/// Pure, so the rule can be tested without a running application. A layout is
/// always produced: with an empty catalog the result is the default workspace
/// with no panels, which the dock renders as an empty area rather than a panic.
pub fn default_layout_from(catalog: &[CatalogPanel]) -> WorkspaceLayout {
    let ordered = ordered(catalog);
    let (main, rest) = ordered.split_at(ordered.len().min(1));

    let mut root = LayoutNode::tabs();
    for panel in main {
        root = root.panel(panel.panel_type.clone());
    }
    if !rest.is_empty() {
        let mut secondary = LayoutNode::tabs();
        for panel in rest {
            secondary = secondary.panel(panel.panel_type.clone());
        }
        root = LayoutNode::h_split()
            .child(root, Some(px(MAIN_COLUMN)))
            .child(secondary, None);
    }

    WorkspaceLayout::new(DEFAULT_WORKSPACE_ID, root)
}

/// Preferred panels first (in [`PREFERRED`] order), then everything else in
/// registration order.
fn ordered(catalog: &[CatalogPanel]) -> Vec<&CatalogPanel> {
    let mut placed: Vec<&CatalogPanel> = Vec::with_capacity(catalog.len());
    for preferred in PREFERRED {
        if let Some(panel) = catalog
            .iter()
            .find(|panel| panel.panel_type.as_str() == *preferred)
        {
            placed.push(panel);
        }
    }
    for panel in catalog {
        if !placed
            .iter()
            .any(|placed| placed.panel_type == panel.panel_type)
        {
            placed.push(panel);
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(plugin: &str, segment: &str) -> CatalogPanel {
        CatalogPanel {
            plugin: PluginId::parse(plugin).expect("the test plugin id is valid"),
            plugin_name: SharedString::from(plugin),
            panel_type: PanelTypeId::parse(format!("{plugin}.{segment}"))
                .expect("the test panel type is valid"),
            title: SharedString::from(segment),
            multiplicity: PanelMultiplicity::Single,
        }
    }

    fn ids(layout: &WorkspaceLayout) -> Vec<String> {
        fn walk(node: &LayoutNode, out: &mut Vec<String>) {
            match node {
                LayoutNode::Tabs { panels, .. } => {
                    out.extend(panels.iter().map(|panel| panel.as_str().to_owned()))
                }
                LayoutNode::Split { children, .. } => {
                    for (child, _) in children {
                        walk(child, out);
                    }
                }
            }
        }
        let mut found = Vec::new();
        walk(layout.root(), &mut found);
        found
    }

    #[test]
    fn a_first_party_contribution_leads_and_the_rest_follow() {
        let catalog = [
            panel("brucebox.git", "diff"),
            panel("brucebox.agent-chat", "chat"),
            panel("brucebox.git", "repository"),
        ];

        let layout = default_layout_from(&catalog);
        let ids = ids(&layout);

        assert_eq!(
            ids,
            vec![
                "brucebox.agent-chat.chat",
                "brucebox.git.repository",
                "brucebox.git.diff"
            ],
            "preferred order, then registration order"
        );
        assert_eq!(layout.id().as_str(), DEFAULT_WORKSPACE_ID);
    }

    #[test]
    fn a_plugin_nobody_anticipated_is_still_placed() {
        let catalog = [
            panel("com.example.hello", "hello"),
            panel("brucebox.agent-chat", "chat"),
        ];

        let ids = ids(&default_layout_from(&catalog));

        assert_eq!(
            ids,
            vec!["brucebox.agent-chat.chat", "com.example.hello.hello"]
        );
    }

    #[test]
    fn a_missing_preferred_plugin_is_skipped_not_asserted() {
        // Git is not installed: the preset must still produce a usable layout.
        let catalog = [panel("brucebox.agent-chat", "chat")];

        let ids = ids(&default_layout_from(&catalog));

        assert_eq!(ids, vec!["brucebox.agent-chat.chat"]);
    }

    #[test]
    fn an_empty_catalog_produces_an_empty_workspace_rather_than_a_panic() {
        let layout = default_layout_from(&[]);

        assert!(ids(&layout).is_empty());
        assert_eq!(layout.id().as_str(), DEFAULT_WORKSPACE_ID);
    }

    #[test]
    fn a_single_contribution_is_not_split_into_two_columns() {
        let catalog = [panel("brucebox.git", "repository")];

        let layout = default_layout_from(&catalog);

        assert!(
            matches!(layout.root(), LayoutNode::Tabs { .. }),
            "one panel is one tab group: {:?}",
            layout.root()
        );
    }

    #[test]
    fn the_shipped_contributions_place_every_panel_exactly_once() {
        // The real registration surface, without a GPUI application: the shipped
        // contribution table is the same data the runtime declares.
        let plugin = PluginId::parse("brucebox.demo").expect("the host plugin id is valid");
        let catalog: Vec<CatalogPanel> = super::super::contributions::proving_panels(&plugin)
            .into_iter()
            .map(|descriptor| CatalogPanel {
                plugin: plugin.clone(),
                plugin_name: SharedString::from("Brucebox"),
                panel_type: descriptor.panel_type().clone(),
                title: descriptor.title().clone(),
                multiplicity: descriptor.multiplicity(),
            })
            .collect();

        let ids = ids(&default_layout_from(&catalog));

        assert_eq!(
            ids.len(),
            catalog.len(),
            "every contribution is placed once"
        );
        for panel in &catalog {
            assert!(
                ids.iter().any(|id| id == panel.panel_type.as_str()),
                "the preset places `{}`",
                panel.panel_type.as_str()
            );
        }
    }

    #[test]
    fn the_shipped_preferred_ids_are_all_valid_panel_types() {
        for id in PREFERRED {
            PanelTypeId::parse(id).expect("every preferred id is a valid panel type id");
        }
    }
}

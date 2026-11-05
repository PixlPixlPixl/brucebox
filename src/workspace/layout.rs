//! Workspace layout data and the plan that resolves it into panels.
//!
//! A layout describes *shape*: splits, tab groups, and the panel type ids that
//! belong in them. It contains no entities and no panel implementations, so it
//! can be built, inspected, and tested without a window.
//!
//! Resolving a layout against the catalog produces a [`WorkspacePlan`]. Planning
//! is where identity, plugin availability, and multiplicity are decided; it
//! never creates a panel. The workspace turns the plan into entities and into
//! GPUI Base Dock's own [`DockLayout`](gpui_kit::base::dock::DockLayout). A
//! panel type that cannot be planned becomes a placeholder carrying a reason
//! instead of silently disappearing, which is what keeps a user's layout
//! recoverable when a plugin is missing.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use gpui_kit::{Pixels, SharedString, px};

use crate::workspace::catalog::{PanelCatalog, PanelDescriptor, PanelInstanceId, PanelTypeId};
use crate::workspace::ids::PanelIdentity;

/// Which way a split lays its children out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    /// Children side by side.
    Horizontal,
    /// Children stacked.
    Vertical,
}

impl SplitAxis {
    fn as_dock_axis(self) -> gpui_kit::Axis {
        match self {
            Self::Horizontal => gpui_kit::Axis::Horizontal,
            Self::Vertical => gpui_kit::Axis::Vertical,
        }
    }
}

/// One node of a layout: either a split or a tab group.
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutNode {
    Split {
        axis: SplitAxis,
        children: Vec<(LayoutNode, Option<Pixels>)>,
    },
    Tabs {
        panels: Vec<PanelTypeId>,
        active: usize,
    },
}

impl LayoutNode {
    /// A split with children side by side.
    pub fn h_split() -> Self {
        Self::Split {
            axis: SplitAxis::Horizontal,
            children: Vec::new(),
        }
    }

    /// A split with children stacked.
    pub fn v_split() -> Self {
        Self::Split {
            axis: SplitAxis::Vertical,
            children: Vec::new(),
        }
    }

    /// An empty tab group.
    pub fn tabs() -> Self {
        Self::Tabs {
            panels: Vec::new(),
            active: 0,
        }
    }

    /// Add a child to a split. `size` is the slot along the split axis.
    pub fn child(mut self, child: LayoutNode, size: Option<Pixels>) -> Self {
        debug_assert!(
            matches!(self, Self::Split { .. }),
            "child() is only valid on h_split() or v_split()"
        );
        if let Self::Split { children, .. } = &mut self {
            children.push((child, size));
        }
        self
    }

    /// Add a panel type to a tab group.
    pub fn panel(mut self, panel_type: PanelTypeId) -> Self {
        debug_assert!(
            matches!(self, Self::Tabs { .. }),
            "panel() is only valid on tabs()"
        );
        if let Self::Tabs { panels, .. } = &mut self {
            panels.push(panel_type);
        }
        self
    }

    /// Which tab is displayed. Out-of-range values are clamped when the dock
    /// normalizes the installed layout.
    pub fn active_index(mut self, index: usize) -> Self {
        debug_assert!(
            matches!(self, Self::Tabs { .. }),
            "active_index() is only valid on tabs()"
        );
        if let Self::Tabs { active, .. } = &mut self {
            *active = index;
        }
        self
    }
}

/// A named workspace arrangement of panel type ids.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceLayout {
    id: SharedString,
    root: LayoutNode,
}

impl WorkspaceLayout {
    /// Wrap a layout tree under a workspace id.
    pub fn new(id: impl Into<SharedString>, root: LayoutNode) -> Self {
        Self {
            id: id.into(),
            root,
        }
    }

    /// The workspace this layout belongs to.
    pub fn id(&self) -> &SharedString {
        &self.id
    }

    /// The layout tree.
    pub fn root(&self) -> &LayoutNode {
        &self.root
    }
}

/// Something the host could not honour while planning a layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceDiagnostic {
    /// No contribution is registered for this panel type.
    UnknownPanelType { panel_type: PanelTypeId },
    /// A single-instance panel type appeared more than once.
    DuplicateSingleInstance { panel_type: PanelTypeId },
    /// The contribution is registered but its factory failed.
    PanelCreateFailed {
        panel_type: PanelTypeId,
        reason: SharedString,
    },
    /// A stored workspace could not be restored, so the default layout was used.
    RestoreFailed { reason: SharedString },
    /// A workspace document could not be written.
    SaveFailed { reason: SharedString },
}

impl fmt::Display for WorkspaceDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPanelType { panel_type } => write!(
                f,
                "panel `{panel_type}` is unavailable: no plugin contribution is registered for it"
            ),
            Self::DuplicateSingleInstance { panel_type } => write!(
                f,
                "panel `{panel_type}` supports a single instance, so the extra placement was replaced by a placeholder"
            ),
            Self::PanelCreateFailed { panel_type, reason } => {
                write!(f, "panel `{panel_type}` could not be created: {reason}")
            }
            Self::RestoreFailed { reason } => write!(
                f,
                "the saved workspace could not be restored ({reason}); the default layout was used"
            ),
            Self::SaveFailed { reason } => {
                write!(f, "the workspace layout could not be saved: {reason}")
            }
        }
    }
}

/// One panel a layout asked for, and whether it can be built.
pub struct PlannedPanel {
    identity: PanelIdentity,
    descriptor: Option<Rc<PanelDescriptor>>,
    unavailable: Option<SharedString>,
}

impl PlannedPanel {
    /// The type and instance id reserved for this panel.
    pub fn identity(&self) -> &PanelIdentity {
        &self.identity
    }

    /// The contribution that will build the panel, when one is available.
    pub fn descriptor(&self) -> Option<&Rc<PanelDescriptor>> {
        self.descriptor.as_ref()
    }

    /// Why no contribution will build this panel, when one is unavailable.
    pub fn unavailable(&self) -> Option<&SharedString> {
        self.unavailable.as_ref()
    }
}

/// A layout resolved against the catalog.
pub struct WorkspacePlan {
    root: PlanNode,
    diagnostics: Vec<WorkspaceDiagnostic>,
}

impl WorkspacePlan {
    /// The resolved layout tree.
    pub fn root(&self) -> &PlanNode {
        &self.root
    }

    /// Problems found while planning, in the order they were found.
    pub fn diagnostics(&self) -> &[WorkspaceDiagnostic] {
        &self.diagnostics
    }

    /// Append diagnostics found while executing the plan.
    pub fn push_diagnostics(&mut self, diagnostics: &[WorkspaceDiagnostic]) {
        self.diagnostics.extend_from_slice(diagnostics);
    }
}

/// A resolved layout node.
pub enum PlanNode {
    Split {
        axis: gpui_kit::Axis,
        children: Vec<(PlanNode, Option<Pixels>)>,
    },
    Tabs {
        panels: Vec<PlannedPanel>,
        active: usize,
    },
}

/// Resolve a layout against the catalog.
///
/// Every panel type gets an instance id even when no contribution can build it:
/// the id is what a later persistence layer writes down, so a placeholder and
/// the panel that eventually replaces it are the same instance.
pub fn plan(layout: &WorkspaceLayout, catalog: &PanelCatalog) -> WorkspacePlan {
    let mut planner = Planner {
        catalog,
        diagnostics: Vec::new(),
        open_instances: HashMap::new(),
    };
    let root = planner.plan_node(layout.root());
    WorkspacePlan {
        root,
        diagnostics: planner.diagnostics,
    }
}

struct Planner<'a> {
    catalog: &'a PanelCatalog,
    diagnostics: Vec<WorkspaceDiagnostic>,
    open_instances: HashMap<PanelTypeId, usize>,
}

impl Planner<'_> {
    fn plan_node(&mut self, node: &LayoutNode) -> PlanNode {
        match node {
            LayoutNode::Split { axis, children } => PlanNode::Split {
                axis: axis.as_dock_axis(),
                children: children
                    .iter()
                    .map(|(child, size)| (self.plan_node(child), *size))
                    .collect(),
            },
            LayoutNode::Tabs { panels, active } => PlanNode::Tabs {
                panels: panels.iter().map(|panel| self.plan_panel(panel)).collect(),
                active: *active,
            },
        }
    }

    fn plan_panel(&mut self, panel_type: &PanelTypeId) -> PlannedPanel {
        let identity = PanelIdentity::new(panel_type.clone(), PanelInstanceId::next());

        let Some(descriptor) = self.catalog.panel(panel_type) else {
            self.diagnostics
                .push(WorkspaceDiagnostic::UnknownPanelType {
                    panel_type: panel_type.clone(),
                });
            return PlannedPanel {
                identity,
                descriptor: None,
                unavailable: Some(
                    "The plugin that provides this panel is not loaded, or failed to load."
                        .to_owned()
                        .into(),
                ),
            };
        };

        let open = self.open_instances.entry(panel_type.clone()).or_insert(0);
        if !descriptor.allows_additional_instance(*open) {
            self.diagnostics
                .push(WorkspaceDiagnostic::DuplicateSingleInstance {
                    panel_type: panel_type.clone(),
                });
            return PlannedPanel {
                identity,
                descriptor: None,
                unavailable: Some(
                    "This panel type supports a single instance, and one is already open."
                        .to_owned()
                        .into(),
                ),
            };
        }
        *open += 1;

        PlannedPanel {
            identity,
            descriptor: Some(descriptor.clone()),
            unavailable: None,
        }
    }
}

/// A convenience for preset data: a split that holds one unconstrained child.
pub fn single_child(child: LayoutNode) -> LayoutNode {
    LayoutNode::v_split().child(child, None)
}

/// The minimum a split slot needs so the dock never sees a zero-pixel panel.
pub const MIN_SLOT: Pixels = px(160.);

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::base::dock::PanelView;
    use gpui_kit::{App, Window, px};

    use super::*;
    use crate::workspace::catalog::{
        PanelCreateError, PanelCreateRequest, PanelFactory, PanelMultiplicity, PluginId,
        PluginRegistration,
    };

    fn plugin(id: &str) -> PluginId {
        PluginId::parse(id).expect("test plugin id is valid")
    }

    fn stub_factory() -> Rc<dyn PanelFactory> {
        Rc::new(
            |request: &PanelCreateRequest,
             _: &mut Window,
             _: &mut App|
             -> Result<Arc<dyn PanelView>, PanelCreateError> {
                Err(PanelCreateError::factory_failed(
                    request.panel_type(),
                    "stub factory",
                ))
            },
        )
    }

    fn catalog() -> PanelCatalog {
        let mut catalog = PanelCatalog::new();
        let owner = plugin("brucebox.git");
        catalog
            .register_plugin(PluginRegistration::new(owner.clone(), "Git"))
            .unwrap();
        for (name, multiplicity) in [
            ("status", PanelMultiplicity::Single),
            ("diff", PanelMultiplicity::Multiple),
        ] {
            let descriptor = PanelDescriptor::new(owner.clone(), name, name, stub_factory())
                .unwrap()
                .with_multiplicity(multiplicity);
            catalog.register_panel(Rc::new(descriptor)).unwrap();
        }
        catalog
    }

    fn panel_type(id: &str) -> PanelTypeId {
        PanelTypeId::parse(id).expect("test panel type is valid")
    }

    fn identities(panels: &[PlannedPanel]) -> Vec<u64> {
        panels
            .iter()
            .map(|panel| panel.identity().instance().get())
            .collect()
    }

    #[test]
    fn planning_reserves_one_instance_per_placement_with_its_own_id() {
        let catalog = catalog();
        let layout = WorkspaceLayout::new(
            "test",
            LayoutNode::h_split()
                .child(
                    LayoutNode::tabs().panel(panel_type("brucebox.git.status")),
                    Some(px(320.)),
                )
                .child(
                    LayoutNode::tabs()
                        .panel(panel_type("brucebox.git.diff"))
                        .panel(panel_type("brucebox.git.diff"))
                        .active_index(1),
                    None,
                ),
        );

        let plan = plan(&layout, &catalog);
        assert!(plan.diagnostics().is_empty(), "{:?}", plan.diagnostics());

        let PlanNode::Split { axis, children } = plan.root() else {
            panic!("the layout root is a split");
        };
        assert_eq!(*axis, gpui_kit::Axis::Horizontal);
        assert_eq!(children.len(), 2);

        let (PlanNode::Tabs { panels, .. }, _) = (&children[0].0, &children[0].1) else {
            panic!("the first child is a tab group");
        };
        assert_eq!(
            panels[0].identity().panel_type(),
            &panel_type("brucebox.git.status")
        );
        assert!(panels[0].unavailable().is_none());

        let (PlanNode::Tabs { panels, active }, _) = (&children[1].0, &children[1].1) else {
            panic!("the second child is a tab group");
        };
        assert_eq!(*active, 1);
        let ids = identities(panels);
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1], "instances of one type get distinct ids");
        assert_eq!(
            panels[0].identity().panel_type(),
            panels[1].identity().panel_type()
        );
    }

    #[test]
    fn an_unknown_panel_type_becomes_a_placeholder_instead_of_vanishing() {
        let catalog = catalog();
        let missing = panel_type("brucebox.agent-chat.chat");
        let layout = WorkspaceLayout::new("test", LayoutNode::tabs().panel(missing.clone()));

        let plan = plan(&layout, &catalog);

        assert_eq!(
            plan.diagnostics(),
            &[WorkspaceDiagnostic::UnknownPanelType {
                panel_type: missing.clone()
            }]
        );
        let PlanNode::Tabs { panels, .. } = plan.root() else {
            panic!("the layout root is a tab group");
        };
        assert_eq!(panels.len(), 1);
        assert!(panels[0].descriptor().is_none());
        assert!(panels[0].unavailable().is_some());
        assert_eq!(panels[0].identity().panel_type(), &missing);
        assert!(
            panels[0].identity().instance().get() > 0,
            "a placeholder still reserves a real instance id"
        );
    }

    #[test]
    fn a_single_instance_panel_is_placed_only_once() {
        let catalog = catalog();
        let single = panel_type("brucebox.git.status");
        let layout = WorkspaceLayout::new(
            "test",
            LayoutNode::h_split()
                .child(LayoutNode::tabs().panel(single.clone()), None)
                .child(LayoutNode::tabs().panel(single.clone()), None),
        );

        let plan = plan(&layout, &catalog);

        assert_eq!(
            plan.diagnostics(),
            &[WorkspaceDiagnostic::DuplicateSingleInstance { panel_type: single }]
        );
        let PlanNode::Split { children, .. } = plan.root() else {
            panic!("the layout root is a split");
        };
        let PlanNode::Tabs { panels, .. } = &children[0].0 else {
            panic!("the first child is a tab group");
        };
        assert!(panels[0].descriptor().is_some());
        let PlanNode::Tabs { panels, .. } = &children[1].0 else {
            panic!("the second child is a tab group");
        };
        assert!(panels[0].descriptor().is_none());
        assert!(panels[0].unavailable().is_some());
    }

    #[test]
    fn a_multiple_instance_panel_may_be_placed_more_than_once() {
        let catalog = catalog();
        let multiple = panel_type("brucebox.git.diff");
        let layout = WorkspaceLayout::new(
            "test",
            LayoutNode::tabs().panel(multiple.clone()).panel(multiple),
        );

        let plan = plan(&layout, &catalog);

        assert!(plan.diagnostics().is_empty(), "{:?}", plan.diagnostics());
        let PlanNode::Tabs { panels, .. } = plan.root() else {
            panic!("the layout root is a tab group");
        };
        assert_eq!(panels.len(), 2);
        assert!(
            panels
                .iter()
                .all(|panel| panel.descriptor().is_some() && panel.unavailable().is_none())
        );
        assert_eq!(identities(panels).len(), 2);
    }

    #[test]
    fn layout_builders_reject_misuse_only_in_debug_builds() {
        // The builders mirror GPUI Base Dock's: they assert in debug and ignore
        // in release, so this only guards the happy path.
        let layout = WorkspaceLayout::new(
            "test",
            LayoutNode::tabs()
                .panel(panel_type("brucebox.git.status"))
                .active_index(0),
        );
        match layout.root() {
            LayoutNode::Tabs { panels, active } => {
                assert_eq!(panels.len(), 1);
                assert_eq!(*active, 0);
            }
            LayoutNode::Split { .. } => panic!("expected a tab group"),
        }
        assert_eq!(layout.id(), "test");
        assert_eq!(
            single_child(LayoutNode::tabs()),
            LayoutNode::v_split().child(LayoutNode::tabs(), None)
        );
        assert!(MIN_SLOT > px(0.));
    }
}

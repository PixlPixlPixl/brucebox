//! Workspace construction: turn a layout into a live GPUI Base Dock area.
//!
//! This module owns the dock area and knows nothing about concrete panels. It
//! plans the layout against the catalog, asks each contribution for a panel,
//! and gives the result to the dock. A panel type that cannot be built becomes
//! a placeholder, so a missing plugin never erases the user's arrangement.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::dock::{DockArea, DockLayout, PanelView};
use gpui_kit::*;

use crate::workspace::catalog;
use crate::workspace::layout::{
    PlanNode, PlannedPanel, WorkspaceDiagnostic, WorkspaceLayout, WorkspacePlan,
};
use crate::workspace::panels::missing::MissingPanel;
use crate::workspace::presentation::debug::DebugDockRenderer;
use crate::workspace::preset::{WORKSPACE_SCHEMA_VERSION, default_layout};

pub struct BruceboxWorkspace {
    dock_area: Entity<DockArea>,
    plan: WorkspacePlan,
}

impl BruceboxWorkspace {
    /// Build the workspace a fresh session starts with.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_layout(default_layout(), window, cx)
    }

    /// Build the workspace from an explicit layout.
    pub fn with_layout(
        layout: WorkspaceLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let panel_count = catalog::try_catalog(cx)
            .map(|catalog| catalog.len())
            .unwrap_or_default();
        assert!(
            panel_count > 0,
            "the panel catalog must be initialised and populated before the workspace is built"
        );

        let mut plan = {
            // Planning only needs to read the catalog, so the read is finished
            // before any panel is created.
            let catalog = catalog::catalog(cx);
            crate::workspace::layout::plan(&layout, catalog)
        };

        let mut diagnostics = Vec::new();
        let dock_layout = build_dock_layout(plan.root(), &mut diagnostics, window, cx);
        plan.push_diagnostics(&diagnostics);

        let dock_area = cx.new(|cx| {
            DockArea::new(
                layout.id().clone(),
                Some(WORKSPACE_SCHEMA_VERSION),
                window,
                cx,
            )
            .with_renderer(Rc::new(DebugDockRenderer))
        });

        dock_area.update(cx, |area, cx| {
            area.set_center(dock_layout, window, cx);
        });

        Self { dock_area, plan }
    }

    /// The dock area this workspace renders.
    pub fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// Problems found while building the workspace, such as a missing plugin.
    pub fn diagnostics(&self) -> &[WorkspaceDiagnostic] {
        self.plan.diagnostics()
    }
}

impl Render for BruceboxWorkspace {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.dock_area.clone())
    }
}

fn build_dock_layout(
    node: &PlanNode,
    diagnostics: &mut Vec<WorkspaceDiagnostic>,
    window: &mut Window,
    cx: &mut App,
) -> DockLayout {
    match node {
        PlanNode::Split { axis, children } => {
            let layout = match axis {
                Axis::Horizontal => DockLayout::h_split(),
                _ => DockLayout::v_split(),
            };
            children.iter().fold(layout, |layout, (child, size)| {
                layout.child(build_dock_layout(child, diagnostics, window, cx), *size)
            })
        }
        PlanNode::Tabs { panels, active } => {
            let layout = panels.iter().fold(DockLayout::tabs(), |layout, panel| {
                let view = build_panel(panel, diagnostics, window, cx);
                layout.panel_view(view, cx)
            });
            layout.active_index(*active)
        }
    }
}

fn build_panel(
    panel: &PlannedPanel,
    diagnostics: &mut Vec<WorkspaceDiagnostic>,
    window: &mut Window,
    cx: &mut App,
) -> Arc<dyn PanelView> {
    let panel_type = panel.identity().panel_type();

    let Some(descriptor) = panel.descriptor() else {
        let detail = panel
            .unavailable()
            .cloned()
            .unwrap_or_else(|| "No plugin contribution is registered for this panel.".into());
        return MissingPanel::for_type_view(panel_type, detail, cx);
    };

    match descriptor.create(panel.identity().instance(), None, window, cx) {
        Ok(view) => view,
        Err(error) => {
            let detail = error.to_string();
            diagnostics.push(WorkspaceDiagnostic::PanelCreateFailed {
                panel_type: panel_type.clone(),
                reason: detail.clone().into(),
            });
            MissingPanel::for_type_view(panel_type, detail, cx)
        }
    }
}

//! Workspace construction and persistence: turn a layout into a live GPUI Base
//! Dock area, and keep the user's arrangement across restarts.
//!
//! This module owns the dock area and knows nothing about concrete panels. It
//! plans a layout against the catalog, asks each contribution for a panel, and
//! gives the result to the dock. A panel type that cannot be built becomes a
//! placeholder, so a missing plugin never erases the user's arrangement.
//!
//! # Lifecycle
//!
//! - A stored [`WorkspaceDocument`] is restored when there is one; the default
//!   preset is only used on first launch, or after an explicit reset, or when
//!   the stored document cannot be restored at all.
//! - Every layout edit schedules a debounced save. Only the newest scheduled
//!   save writes, so a drag that fires dozens of `LayoutChanged` events costs
//!   one file write.
//! - Quitting flushes synchronously, so a change made just before exit is not
//!   lost to the debounce window.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::dock::{DockArea, DockEvent, DockLayout, PanelView};
use gpui_kit::*;

use crate::workspace::ResetWorkspace;
use crate::workspace::catalog;
use crate::workspace::document::WorkspaceDocument;
use crate::workspace::layout::{PlanNode, PlannedPanel, WorkspaceDiagnostic, WorkspaceLayout};
use crate::workspace::panels::missing::MissingPanel;
use crate::workspace::presentation::debug::DebugDockRenderer;
use crate::workspace::preset::{WORKSPACE_SCHEMA_VERSION, default_layout};
use crate::workspace::store::WorkspaceStore;

/// How long a layout edit waits before it is written to disk.
///
/// Long enough to coalesce a drag, short enough that a user who quits right
/// after rearranging still has the change flushed by the quit hook.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);

pub struct BruceboxWorkspace {
    workspace_id: SharedString,
    dock_area: Entity<DockArea>,
    store: WorkspaceStore,
    diagnostics: Vec<WorkspaceDiagnostic>,
    save_generation: u64,
    focus_handle: FocusHandle,
    _dock_events: Subscription,
    _window_closed: Subscription,
    _app_quit: Subscription,
}

impl BruceboxWorkspace {
    /// Build the workspace a session starts with.
    ///
    /// Restores the saved layout for the default workspace when one exists.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_layout(default_layout(), window, cx)
    }

    /// Build a workspace from an explicit layout, persisting to the default
    /// location for that workspace id.
    pub fn with_layout(
        layout: WorkspaceLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let store = WorkspaceStore::for_workspace(layout.id().as_str());
        Self::with_store(layout, store, window, cx)
    }

    /// Build a workspace with an explicit layout and store.
    ///
    /// Useful for tests, for alternate data roots, and for a future
    /// "workspace as <path>" feature. An existing document in the store wins
    /// over the supplied layout.
    pub fn with_store(
        layout: WorkspaceLayout,
        store: WorkspaceStore,
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

        let workspace_id = layout.id().clone();
        let mut diagnostics = Vec::new();

        // Read the stored document before touching the dock: it decides whether
        // the default preset is built at all.
        let stored = match store.load() {
            Ok(document) => document,
            Err(error) => {
                diagnostics.push(WorkspaceDiagnostic::RestoreFailed {
                    reason: error.to_string().into(),
                });
                store.quarantine();
                None
            }
        };

        let dock_area = cx.new(|cx| {
            DockArea::new(
                workspace_id.clone(),
                Some(WORKSPACE_SCHEMA_VERSION),
                window,
                cx,
            )
            .with_renderer(Rc::new(DebugDockRenderer))
        });

        let mut restored = false;
        if let Some(document) = stored {
            match document.validate(workspace_id.as_str()) {
                Ok(()) => {
                    let state = document.into_dock();
                    match dock_area.update(cx, |area, cx| area.load(state, window, cx)) {
                        Ok(()) => restored = true,
                        Err(error) => {
                            diagnostics.push(WorkspaceDiagnostic::RestoreFailed {
                                reason: error.to_string().into(),
                            });
                            store.quarantine();
                        }
                    }
                }
                Err(error) => {
                    diagnostics.push(WorkspaceDiagnostic::RestoreFailed {
                        reason: error.to_string().into(),
                    });
                    store.quarantine();
                }
            }
        }

        if !restored {
            let (dock_layout, plan_diagnostics) = build_default_layout(&layout, window, cx);
            diagnostics.extend(plan_diagnostics);
            dock_area.update(cx, |area, cx| {
                area.set_center(dock_layout, window, cx);
            });
        }

        // Subscribe only now: constructing the layout above emits
        // `LayoutChanged`, and that is not a user edit worth saving.
        let dock_events = cx.subscribe(&dock_area, |this, _, _: &DockEvent, cx| {
            this.schedule_save(cx);
        });

        // Closing the window drops the root view, and with it this workspace,
        // before the app-quit observers run. The window-closed hook fires while
        // the entities are still alive, so it is what makes a change made
        // inside the debounce window survive an immediate quit. It captures the
        // dock area and store directly instead of going through `Self`.
        let window_closed = cx.on_window_closed({
            let store = store.clone();
            let dock_area = dock_area.clone();
            let workspace_id = workspace_id.clone();
            move |cx, _window_id| {
                persist(&store, workspace_id.as_str(), &dock_area, cx);
            }
        });

        let app_quit = cx.on_app_quit(|this, cx| {
            this.flush(cx);
            async {}
        });

        let focus_handle = cx.focus_handle();
        let workspace = Self {
            workspace_id,
            dock_area,
            store,
            diagnostics,
            save_generation: 0,
            focus_handle: focus_handle.clone(),
            _dock_events: dock_events,
            _window_closed: window_closed,
            _app_quit: app_quit,
        };

        // The workspace root owns focus until a panel claims it, so workspace
        // level actions (the reset shortcut) dispatch without a click first.
        focus_handle.focus(window, cx);
        workspace
    }

    /// The dock area this workspace renders.
    pub fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// The workspace id this document is saved under.
    pub fn workspace_id(&self) -> &SharedString {
        &self.workspace_id
    }

    /// Where this workspace is persisted.
    pub fn store(&self) -> &WorkspaceStore {
        &self.store
    }

    /// Problems found while building or saving the workspace.
    pub fn diagnostics(&self) -> &[WorkspaceDiagnostic] {
        &self.diagnostics
    }

    /// Replace the layout with the default preset and persist that.
    ///
    /// This is the explicit reset: the saved document is removed and the
    /// default arrangement is written in its place.
    pub fn reset_to_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let layout = default_layout();
        let (dock_layout, plan_diagnostics) = build_default_layout(&layout, window, cx);

        self.dock_area.update(cx, |area, cx| {
            area.set_center(dock_layout, window, cx);
        });

        self.diagnostics
            .retain(|diagnostic| !matches!(diagnostic, WorkspaceDiagnostic::SaveFailed { .. }));
        self.diagnostics.extend(plan_diagnostics);

        if let Err(error) = self.store.remove() {
            self.record_save_error(error.to_string(), cx);
        }
        self.flush(cx);
        cx.notify();
    }

    /// The document a save would write.
    fn document(&self, cx: &App) -> WorkspaceDocument {
        WorkspaceDocument::current(
            self.workspace_id.to_string(),
            self.dock_area.read(cx).dump(cx),
        )
    }

    /// Schedule a debounced save, replacing any save already scheduled.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        self.save_generation = self.save_generation.wrapping_add(1);
        let generation = self.save_generation;
        let store = self.store.clone();

        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_DEBOUNCE).await;

            // Dump on the GPUI thread; only the newest scheduled save may win.
            let payload = this.update(cx, |workspace, cx| {
                (workspace.save_generation == generation).then(|| workspace.document(cx).to_json())
            });

            let json = match payload {
                Ok(Some(Ok(json))) => json,
                Ok(Some(Err(error))) => {
                    let reason = error.to_string();
                    this.update(cx, |workspace, cx| {
                        workspace.record_save_error(reason, cx);
                    })
                    .ok();
                    return;
                }
                // Superseded by a newer save, or the workspace is gone.
                Ok(None) | Err(_) => return,
            };

            let written = cx
                .background_spawn(async move { store.save_json(&json) })
                .await;
            if let Err(error) = written {
                let reason = error.to_string();
                this.update(cx, |workspace, cx| {
                    workspace.record_save_error(reason, cx);
                })
                .ok();
            }
        })
        .detach();
    }

    /// Write the current layout immediately, cancelling any pending debounce.
    fn flush(&mut self, cx: &mut Context<Self>) {
        self.save_generation = self.save_generation.wrapping_add(1);
        let document = self.document(cx);

        if let Err(error) = self.store.save(&document) {
            self.record_save_error(error.to_string(), cx);
        }
    }

    fn record_save_error(&mut self, reason: String, cx: &mut Context<Self>) {
        self.diagnostics
            .retain(|diagnostic| !matches!(diagnostic, WorkspaceDiagnostic::SaveFailed { .. }));
        self.diagnostics.push(WorkspaceDiagnostic::SaveFailed {
            reason: reason.into(),
        });
        cx.notify();
    }
}

/// Write the dock's current layout without an entity context.
///
/// Used by the window-closed hook, which runs before the workspace is dropped
/// and cannot reach it. Failures are reported on stderr: there is no window left
/// to show a diagnostic in.
fn persist(store: &WorkspaceStore, workspace_id: &str, dock_area: &Entity<DockArea>, cx: &App) {
    let document = WorkspaceDocument::current(workspace_id.to_owned(), dock_area.read(cx).dump(cx));
    if let Err(error) = store.save(&document) {
        eprintln!("brucebox: workspace `{workspace_id}` could not be saved: {error}");
    }
}

impl Render for BruceboxWorkspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context("BruceboxWorkspace")
            .on_action(cx.listener(|this, _: &ResetWorkspace, window, cx| {
                this.reset_to_default(window, cx);
            }))
            .child(self.dock_area.clone())
    }
}

/// Plan a layout and turn it into a `DockLayout`, collecting diagnostics.
fn build_default_layout(
    layout: &WorkspaceLayout,
    window: &mut Window,
    cx: &mut App,
) -> (DockLayout, Vec<WorkspaceDiagnostic>) {
    let mut plan = {
        // Planning only reads the catalog, so the read finishes before any
        // panel is created.
        let catalog = catalog::catalog(cx);
        crate::workspace::layout::plan(layout, catalog)
    };

    let mut diagnostics = Vec::new();
    let dock_layout = build_dock_layout(plan.root(), &mut diagnostics, window, cx);
    plan.push_diagnostics(&diagnostics);
    (dock_layout, plan.diagnostics().to_vec())
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

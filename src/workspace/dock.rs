//! Workspace construction and persistence: turn a layout into a live GPUI Base
//! Dock area, keep the user's arrangement across restarts, and layer floating
//! surfaces over it.
//!
//! This module owns the dock areas and knows nothing about concrete panels. It
//! plans a layout against the catalog, asks each contribution for a panel, and
//! gives the result to the dock. A panel type that cannot be built becomes a
//! placeholder, so a missing plugin never erases the user's arrangement.
//!
//! # Shape
//!
//! - The **docked workspace** is one GPUI Base Dock area filling the window.
//! - Each **floating surface** is another dock area, drawn above the docked one
//!   at its own bounds and in its own stacking position. Panels move between
//!   areas as panel *views*: the same entity, the same identity, the same
//!   state — see [`crate::workspace::panels::PanelHandle`].
//!
//! # Lifecycle
//!
//! - A stored [`WorkspaceDocument`] is restored when there is one; the default
//!   preset is only used on first launch, after an explicit reset, or when the
//!   stored document cannot be restored at all. Restoring is all-or-nothing: a
//!   surface that fails takes the whole document with it, so a half-restored
//!   workspace is never saved back over the real one.
//! - Every layout edit schedules a debounced save. Only the newest scheduled
//!   save writes, so a drag that fires dozens of `LayoutChanged` events costs
//!   one file write.
//! - Quitting flushes synchronously, so a change made just before exit is not
//!   lost to the debounce window.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::dock::{
    DockArea, DockEvent, DockLayout, DockPlacement, PaneRef, PanelId, PanelView,
};
use gpui_kit::*;

use crate::workspace::catalog;
use crate::workspace::document::WorkspaceDocument;
use crate::workspace::floating::{FloatingSurfaceEvent, FloatingSurfaceView};
use crate::workspace::ids::{PanelInstanceId, PanelTypeId};
use crate::workspace::layout::{PlanNode, PlannedPanel, WorkspaceDiagnostic, WorkspaceLayout};
use crate::workspace::panels::PanelHandle;
use crate::workspace::panels::missing::MissingPanel;
use crate::workspace::presentation::debug::DebugDockRenderer;
use crate::workspace::preset::{WORKSPACE_SCHEMA_VERSION, default_layout};
use crate::workspace::store::WorkspaceStore;
use crate::workspace::surface::{FloatingSurfaceState, SurfaceArea, SurfaceBounds, SurfaceId};
use crate::workspace::{DockFloatingSurfaces, FloatActivePanel, OpenPanelCatalog, ResetWorkspace};

/// How long a layout edit waits before it is written to disk.
///
/// Long enough to coalesce a drag, short enough that a user who quits right
/// after rearranging still has the change flushed by the quit hook.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);

/// Which dock area a panel lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AreaRef {
    /// The docked workspace that fills the window.
    Main,
    /// A floating surface, by id.
    Surface(SurfaceId),
}

pub struct BruceboxWorkspace {
    workspace_id: SharedString,
    dock_area: Entity<DockArea>,
    /// Floating surfaces, back to front: the last one is on top.
    surfaces: Vec<SurfaceSlot>,
    store: WorkspaceStore,
    diagnostics: Vec<WorkspaceDiagnostic>,
    save_generation: u64,
    /// How many instances of each panel type are open, kept in step with the dock.
    ///
    /// Cached rather than derived on demand: deriving means dumping the dock, and
    /// dumping a panel reads its entity — which panics if the panel asking is the
    /// one currently being updated (the catalog clicking its own row).
    instances: std::collections::BTreeMap<PanelTypeId, usize>,
    focus_handle: FocusHandle,
    _dock_events: Subscription,
    _window_closed: Subscription,
    _app_quit: Subscription,
}

/// One live floating surface and the subscriptions that keep the workspace in
/// step with it.
///
/// The subscriptions are held rather than dropped: subscribing returns a handle
/// that unsubscribes when it goes away, so a surface whose subscription was not
/// kept would stop reporting its edits.
struct SurfaceSlot {
    id: SurfaceId,
    view: Entity<FloatingSurfaceView>,
    dock_area: Entity<DockArea>,
    _dock_events: Subscription,
    _surface_events: Subscription,
}

impl BruceboxWorkspace {
    /// Build the workspace a session starts with.
    ///
    /// Restores the saved layout for the default workspace when one exists.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with_layout(default_layout(cx), window, cx)
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

        // Restoring is all-or-nothing. A document whose surfaces cannot be
        // rebuilt is not a document this build understands, and saving a
        // half-restored workspace over it would destroy the user's layout.
        let mut surfaces = Vec::new();
        let mut restored = false;
        if let Some(document) = stored {
            match restore_document(
                document,
                &workspace_id,
                &dock_area,
                window,
                cx,
                &mut surfaces,
            ) {
                Ok(()) => restored = true,
                Err(diagnostic) => {
                    diagnostics.push(diagnostic);
                    store.quarantine();
                }
            }
        }

        if !restored {
            surfaces.clear();
            let (dock_layout, plan_diagnostics) = build_default_layout(&layout, window, cx);
            diagnostics.extend(plan_diagnostics);
            dock_area.update(cx, |area, cx| {
                area.set_center(dock_layout, window, cx);
            });
        }

        // Subscribe to the docked area only now: building the layout above
        // emits `LayoutChanged`, and that is not a user edit worth saving.
        let dock_events = cx.subscribe(&dock_area, |this, _, _: &DockEvent, cx| {
            this.recount_instances(cx);
            this.schedule_save(cx);
        });

        // Closing the window drops the root view, and with it this workspace,
        // before the app-quit observers run. The window-closed hook fires while
        // the entities are still alive, so it is what makes a change made
        // inside the debounce window survive an immediate quit. It captures the
        // dock areas and store directly instead of going through `Self`.
        // Closing the window drops the root view, and with it this workspace,
        // before the app-quit observers run. This hook fires while the entities
        // are still alive, so flushing through `Self` is what makes a change
        // made inside the debounce window survive an immediate quit — and what
        // keeps floating surfaces in the saved document, since they come and go
        // long after this hook was registered and a snapshot taken here would
        // miss every one of them.
        let window_closed = {
            let weak = cx.weak_entity();
            cx.on_window_closed(move |cx, _window_id| {
                weak.update(cx, |workspace, cx| workspace.flush(cx)).ok();
            })
        };

        let app_quit = cx.on_app_quit(|this, cx| {
            this.flush(cx);
            async {}
        });

        let focus_handle = cx.focus_handle();
        let mut workspace = Self {
            workspace_id,
            dock_area,
            surfaces,
            store,
            diagnostics,
            save_generation: 0,
            instances: std::collections::BTreeMap::new(),
            focus_handle: focus_handle.clone(),
            _dock_events: dock_events,
            _window_closed: window_closed,
            _app_quit: app_quit,
        };

        // One count for the dock as built or restored, so opening a panel from the
        // catalog knows how many instances already exist.
        let counts = workspace.instance_counts(cx);
        workspace.instances = counts;

        // The workspace root owns focus until a panel claims it, so workspace
        // level actions (the reset and float shortcuts) dispatch without a
        // click first.
        focus_handle.focus(window, cx);
        workspace
    }

    /// The docked area this workspace renders.
    pub fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// The id of every floating surface, back to front.
    pub fn surface_ids(&self) -> Vec<SurfaceId> {
        self.surfaces.iter().map(|slot| slot.id).collect()
    }

    /// The number of live floating surfaces.
    pub fn surface_count(&self) -> usize {
        self.surfaces.len()
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
    /// This is the explicit reset: the floating surfaces and the saved document
    /// are discarded and the default docked arrangement is written in their
    /// place.
    pub fn reset_to_default(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let layout = default_layout(cx);
        let (dock_layout, plan_diagnostics) = build_default_layout(&layout, window, cx);

        self.surfaces.clear();
        self.dock_area.update(cx, |area, cx| {
            area.set_center(dock_layout, window, cx);
        });
        self.recount_instances(cx);

        self.diagnostics.retain(|diagnostic| {
            !matches!(
                diagnostic,
                WorkspaceDiagnostic::SaveFailed { .. }
                    | WorkspaceDiagnostic::TransferFailed { .. }
                    | WorkspaceDiagnostic::RestoreFailed { .. }
            )
        });
        self.diagnostics.extend(plan_diagnostics);

        if let Err(error) = self.store.remove() {
            self.record_save_error(error.to_string(), cx);
        }
        self.flush(cx);
        cx.notify();
    }

    /// How many instances of each panel type are open, as last counted.
    ///
    /// Cheap and safe to call from a panel that is mid-update; the dock event
    /// subscription keeps it current.
    pub fn open_instances(&self) -> &std::collections::BTreeMap<PanelTypeId, usize> {
        &self.instances
    }

    /// Recount open instances from the live dock.
    ///
    /// Must not run while a panel is being updated: dumping the dock reads every
    /// panel entity, including the one calling in.
    fn recount_instances(&mut self, cx: &App) {
        self.instances = self.instance_counts(cx);
    }

    /// How many instances of each panel type this workspace currently holds.
    ///
    /// Read back from the dock itself rather than tracked alongside it, so a
    /// restored document, a floated panel and a panel opened from the catalog all
    /// count the same way.
    pub fn instance_counts(&self, cx: &App) -> std::collections::BTreeMap<PanelTypeId, usize> {
        let document = self.document(cx);
        let mut counts = std::collections::BTreeMap::new();
        count_panels(&document.dock().center, &mut counts);
        for surface in document.surfaces() {
            count_panels(&surface.dock().center, &mut counts);
        }
        counts
    }

    /// Open one registered panel type in the docked workspace.
    ///
    /// This is how the panel catalog opens a contribution, and it is the only
    /// path that creates a panel outside a saved layout or the first-run preset.
    /// The panel is created by its plugin's factory and placed in the centre dock,
    /// where the dock merges it into the first tab group.
    ///
    /// Fails, with a reason for the status view, when nothing registers the type
    /// (its plugin is not installed) or when the type already has all the
    /// instances its declaration allows.
    pub fn open_panel(
        &mut self,
        panel_type: &PanelTypeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let descriptor = catalog::try_catalog(cx)
            .and_then(|catalog| {
                catalog
                    .panels()
                    .find(|descriptor| descriptor.panel_type() == panel_type)
                    .cloned()
            })
            .ok_or_else(|| format!("no plugin registers a panel type `{}`", panel_type.as_str()))?;

        let open = self.instances.get(panel_type).copied().unwrap_or_default();
        if !descriptor.allows_additional_instance(open) {
            return Err(format!(
                "`{}` allows {} instance(s) and {open} is already open",
                panel_type.as_str(),
                descriptor.multiplicity().max_instances().unwrap_or(0)
            ));
        }

        let instance = PanelInstanceId::next();
        let view = descriptor
            .create(instance, None, window, cx)
            .map_err(|error| format!("`{}` could not be built: {error}", panel_type.as_str()))?;

        self.dock_area.update(cx, |area, cx| {
            area.add_panel_view(view, DockPlacement::Center, None, window, cx);
        });
        *self.instances.entry(panel_type.clone()).or_default() += 1;
        // Debounced, not synchronous: opening from a panel runs inside that
        // panel's update, and dumping the dock reads every panel entity —
        // including the one that asked.
        self.schedule_save(cx);
        cx.notify();
        Ok(())
    }

    /// Open this build's panel catalog, if it registers one.
    pub fn open_panel_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let panel_type = catalog::try_catalog(cx).and_then(|catalog| {
            catalog
                .panels()
                .map(|descriptor| descriptor.panel_type().clone())
                .find(|panel_type| {
                    panel_type.as_str() == crate::workspace::contributions::CATALOG_PANEL
                })
        });
        let Some(panel_type) = panel_type else {
            self.record_transfer_error("this build registers no panel catalog", cx);
            return;
        };
        if let Err(reason) = self.open_panel(&panel_type, window, cx) {
            self.record_transfer_error(reason, cx);
        }
    }

    /// Float the active panel, or dock it again if it is already floating.
    ///
    /// "Active" is the panel the user last put focus on inside the area under
    /// the pointer, falling back to the displayed tab of that area's first
    /// group. Using the pointer keeps the command predictable without a
    /// separate "pick a panel" mode.
    pub fn float_active_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((source, view)) = self.active_panel(window, cx) else {
            self.record_transfer_error("there is no active panel to float", cx);
            return;
        };

        match source {
            AreaRef::Main => {
                let Some(id) = self.ensure_surface(window, cx) else {
                    return;
                };
                self.transfer(view, AreaRef::Main, AreaRef::Surface(id), window, cx);
            }
            AreaRef::Surface(id) => {
                self.transfer(view, AreaRef::Surface(id), AreaRef::Main, window, cx);
            }
        }
    }

    /// Bring every floating panel back into the docked workspace.
    pub fn dock_floating_surfaces(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<SurfaceId> = self.surfaces.iter().map(|slot| slot.id).collect();
        for id in ids {
            self.dock_surface(id, window, cx);
        }
    }

    /// Move one surface's panels back into the docked workspace and close it.
    fn dock_surface(&mut self, id: SurfaceId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slot) = self.surfaces.iter().find(|slot| slot.id == id) else {
            return;
        };
        let area = slot.dock_area.clone();

        for panel in panels_in(&area, cx) {
            self.transfer(panel, AreaRef::Surface(id), AreaRef::Main, window, cx);
        }

        self.surfaces.retain(|slot| slot.id != id);
        cx.notify();
    }

    /// The dock area for `area`, if it still exists.
    fn area(&self, area: AreaRef) -> Option<&Entity<DockArea>> {
        match area {
            AreaRef::Main => Some(&self.dock_area),
            AreaRef::Surface(id) => self
                .surfaces
                .iter()
                .find(|slot| slot.id == id)
                .map(|slot| &slot.dock_area),
        }
    }

    /// Move one panel to another area, keeping the panel itself.
    ///
    /// The panel view is added to the destination first and only then removed
    /// from the source, so the panel is never absent from every dock for an
    /// instant — a plugin whose UI reacts to being detached would flicker
    /// otherwise. The removal notice is withheld: this is a move, not a close.
    fn transfer(
        &mut self,
        view: Arc<dyn PanelView>,
        from: AreaRef,
        to: AreaRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if from == to {
            return;
        }

        let (Some(source), Some(target)) = (self.area(from).cloned(), self.area(to).cloned())
        else {
            self.record_transfer_error("the source or destination surface is gone", cx);
            return;
        };

        let Some(handle) = PanelHandle::of(&view) else {
            let reason = format!(
                "panel `{}` was not created by this workspace, so it cannot be moved",
                view.panel_name(cx)
            );
            self.record_transfer_error(reason, cx);
            return;
        };

        handle.begin_transfer(cx);
        target.update(cx, |area, cx| {
            area.add_panel_view(view.clone(), DockPlacement::Center, None, window, cx);
        });
        source.update(cx, |area, cx| handle.remove_from(area, window, cx));
        handle.end_transfer(cx);

        // A surface whose last panel left has nothing left to show.
        if let AreaRef::Surface(id) = from {
            self.remove_if_empty(id, cx);
        }
        cx.notify();
    }

    /// Drop a surface that no longer displays any panel.
    fn remove_if_empty(&mut self, id: SurfaceId, cx: &mut Context<Self>) {
        let Some(index) = self.surfaces.iter().position(|slot| slot.id == id) else {
            return;
        };
        if self.surfaces[index]
            .dock_area
            .read(cx)
            .is_empty(DockPlacement::Center, cx)
        {
            self.surfaces.remove(index);
        }
    }

    /// The surface to float into: the topmost one, or a new one if there is none.
    fn ensure_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<SurfaceId> {
        if let Some(slot) = self.surfaces.last() {
            return Some(slot.id);
        }

        let area = SurfaceArea::new(
            f32::from(window.viewport_size().width),
            f32::from(window.viewport_size().height),
        );
        let id = SurfaceId::next();
        let bounds = SurfaceBounds::cascaded(0, area);
        let dock_area = cx.new(|cx| {
            DockArea::new(
                surface_dock_id(&self.workspace_id, id),
                Some(WORKSPACE_SCHEMA_VERSION),
                window,
                cx,
            )
            .with_renderer(Rc::new(DebugDockRenderer))
        });
        let dock_events = cx.subscribe(&dock_area, |this, _, _: &DockEvent, cx| {
            this.recount_instances(cx);
            this.schedule_save(cx);
        });
        let view =
            FloatingSurfaceView::new(id, format!("Floating {id}"), bounds, dock_area.clone(), cx);
        let surface_events = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &FloatingSurfaceEvent, window, cx| {
                this.on_surface_event(*event, window, cx);
            },
        );

        self.surfaces.push(SurfaceSlot {
            id,
            view,
            dock_area,
            _dock_events: dock_events,
            _surface_events: surface_events,
        });
        cx.notify();
        Some(id)
    }

    fn on_surface_event(
        &mut self,
        event: FloatingSurfaceEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FloatingSurfaceEvent::Raised(id) => self.raise(id, cx),
            FloatingSurfaceEvent::DockRequested(id) => self.dock_surface(id, window, cx),
            FloatingSurfaceEvent::BoundsChanged(_) => self.schedule_save(cx),
        }
    }

    /// Put a surface on top of the others.
    fn raise(&mut self, id: SurfaceId, cx: &mut Context<Self>) {
        let Some(index) = self.surfaces.iter().position(|slot| slot.id == id) else {
            return;
        };
        if index + 1 == self.surfaces.len() {
            return;
        }
        let slot = self.surfaces.remove(index);
        self.surfaces.push(slot);
        cx.notify();
    }

    /// The area under the pointer: the topmost surface there, else the dock.
    fn area_under_pointer(&self, window: &Window, cx: &App) -> AreaRef {
        let position = window.mouse_position();
        let (x, y) = (f32::from(position.x), f32::from(position.y));

        for slot in self.surfaces.iter().rev() {
            if slot.view.read(cx).bounds().contains(x, y) {
                return AreaRef::Surface(slot.id);
            }
        }
        AreaRef::Main
    }

    /// The panel most recently made active in the area under the pointer.
    fn active_panel(&self, window: &Window, cx: &App) -> Option<(AreaRef, Arc<dyn PanelView>)> {
        let area = self.area_under_pointer(window, cx);
        let dock_area = self.area(area)?;

        let view = focused_panel(dock_area, window, cx)
            .or_else(|| first_group_active_panel(dock_area, cx))?;

        Some((area, view))
    }

    /// The document a save would write.
    fn document(&self, cx: &App) -> WorkspaceDocument {
        let surfaces = self
            .surfaces
            .iter()
            .enumerate()
            .map(|(stack, slot)| {
                let view = slot.view.read(cx);
                FloatingSurfaceState::new(
                    slot.id,
                    view.bounds(),
                    stack as u32,
                    slot.dock_area.read(cx).dump(cx),
                )
            })
            .collect();

        WorkspaceDocument::with_surfaces(
            self.workspace_id.to_string(),
            self.dock_area.read(cx).dump(cx),
            surfaces,
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

    fn record_transfer_error(&mut self, reason: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.diagnostics.push(WorkspaceDiagnostic::TransferFailed {
            reason: reason.into(),
        });
        cx.notify();
    }
}

/// Restore a stored document into the freshly created dock areas.
///
/// All-or-nothing: a document whose docked area or any surface cannot be loaded
/// is rejected as a whole, leaving the caller to quarantine it and fall back to
/// the default preset.
fn restore_document(
    document: WorkspaceDocument,
    workspace_id: &SharedString,
    dock_area: &Entity<DockArea>,
    window: &mut Window,
    cx: &mut Context<BruceboxWorkspace>,
    surfaces: &mut Vec<SurfaceSlot>,
) -> Result<(), WorkspaceDiagnostic> {
    document.validate(workspace_id.as_str()).map_err(|error| {
        WorkspaceDiagnostic::RestoreFailed {
            reason: error.to_string().into(),
        }
    })?;

    let (dock_state, surface_states) = document.upgraded().into_parts();

    dock_area
        .update(cx, |area, cx| area.load(dock_state, window, cx))
        .map_err(|error| WorkspaceDiagnostic::RestoreFailed {
            reason: format!("the docked workspace could not be restored: {error}").into(),
        })?;

    let area_size = SurfaceArea::new(
        f32::from(window.viewport_size().width),
        f32::from(window.viewport_size().height),
    );

    // Stacking order is the document's, not the file's: sort before building so
    // the vec order and the persisted order agree.
    let mut ordered = surface_states;
    ordered.sort_by_key(|state| state.z());

    for state in ordered {
        let id = state.id();
        let bounds = state.bounds().clamped(area_size);

        let area = cx.new(|cx| {
            DockArea::new(
                surface_dock_id(workspace_id, id),
                Some(WORKSPACE_SCHEMA_VERSION),
                window,
                cx,
            )
            .with_renderer(Rc::new(DebugDockRenderer))
        });

        area.update(cx, |area, cx| area.load(state.into_dock(), window, cx))
            .map_err(|error| WorkspaceDiagnostic::RestoreFailed {
                reason: format!("floating surface {id} could not be restored: {error}").into(),
            })?;

        let view = FloatingSurfaceView::new(id, format!("Floating {id}"), bounds, area.clone(), cx);
        let dock_events = cx.subscribe(&area, |this, _, _: &DockEvent, cx| {
            this.schedule_save(cx);
        });
        let surface_events = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &FloatingSurfaceEvent, window, cx| {
                this.on_surface_event(*event, window, cx);
            },
        );

        surfaces.push(SurfaceSlot {
            id,
            view,
            dock_area: area,
            _dock_events: dock_events,
            _surface_events: surface_events,
        });
    }

    Ok(())
}

/// The dock name for a floating surface, unique within the workspace.
fn surface_dock_id(workspace_id: &SharedString, id: SurfaceId) -> SharedString {
    format!("{workspace_id}#floating-{id}").into()
}

/// Every panel a dock area currently holds, in tree order.
fn panels_in(area: &Entity<DockArea>, cx: &App) -> Vec<Arc<dyn PanelView>> {
    let area = area.read(cx);
    let Some(tree) = area.layout(DockPlacement::Center) else {
        return Vec::new();
    };

    let ids = tree.panels().collect::<Vec<PanelId>>();
    ids.into_iter()
        .filter_map(|panel| area.panel(panel).cloned())
        .collect()
}

/// The panel that currently holds focus in `area`, if any.
fn focused_panel(area: &Entity<DockArea>, window: &Window, cx: &App) -> Option<Arc<dyn PanelView>> {
    panels_in(area, cx)
        .into_iter()
        .find(|panel| panel.focus_handle(cx).contains_focused(window, cx))
}

/// The displayed tab of the first tab group in `area`.
///
/// A workspace command has no pointer position to work from once the user has
/// moved the mouse to a shortcut, so this is the deterministic fallback.
fn first_group_active_panel(area: &Entity<DockArea>, cx: &App) -> Option<Arc<dyn PanelView>> {
    let area = area.read(cx);
    let tree = area.layout(DockPlacement::Center)?;

    let mut found: Option<PanelId> = None;
    tree.root().walk(&mut |node| {
        if found.is_some() {
            return;
        }
        if let PaneRef::Tabs { panels, active_ix } = node.kind() {
            found = panels.get(active_ix).or_else(|| panels.first()).copied();
        }
    });

    area.panel(found?).cloned()
}

impl Render for BruceboxWorkspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context("BruceboxWorkspace")
            .on_action(cx.listener(|this, _: &ResetWorkspace, window, cx| {
                this.reset_to_default(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FloatActivePanel, window, cx| {
                this.float_active_panel(window, cx);
            }))
            .on_action(cx.listener(|this, _: &DockFloatingSurfaces, window, cx| {
                this.dock_floating_surfaces(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenPanelCatalog, window, cx| {
                this.open_panel_catalog(window, cx);
            }))
            .child(div().absolute().inset_0().child(self.dock_area.clone()))
            .children(self.surfaces.iter().map(|slot| slot.view.clone()))
    }
}

/// Count one persisted dock tree's panels by type.
///
/// Container nodes (`StackPanel`, `TabPanel`) do not parse as panel type ids, so
/// they need no special case.
fn count_panels(
    state: &gpui_kit::base::dock::PanelState,
    counts: &mut std::collections::BTreeMap<PanelTypeId, usize>,
) {
    if let Ok(panel_type) = PanelTypeId::parse(&state.panel_name) {
        *counts.entry(panel_type).or_default() += 1;
    }
    for child in &state.children {
        count_panels(child, counts);
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

# ADR 0003 — Floating surfaces over the docked workspace

- Status: accepted
- Date: 2026-11-05
- Scope: PLAN.md step 3
- Related: SPEC.md §4.4–4.7; ADR 0001, ADR 0002

## Context

GPUI Base Dock gives one dock area per entity: a tree of splits and tab groups,
with tabs moved and dropped inside that one area. Brucebox needs panels that sit
*above* the docked workspace, move and resize freely, overlap each other in a
stacking order, and can go back into the dock — without losing the panel, its
identity, or its state on the way.

## Decisions

### 1. A surface is another dock area, not a new layout system

Each floating surface owns an `Entity<DockArea>`. Panels inside a surface
therefore get exactly the same tabs, splits, resizing and reordering as the main
workspace, and none of that has to be reimplemented or kept compatible by hand.

The alternative — a Brucebox-specific "group of panels" model — would have had
to reproduce tab order, splits, sizes and drag geometry, and would have drifted
from the docked case the first time Base Dock changed.

### 2. The workspace document grows a surface list; the trees stay authoritative

`WorkspaceDocument` schema 2 adds `surfaces: [FloatingSurfaceState]`, where each
entry is `{ id, bounds, z, dock }` and `dock` is a full `DockAreaState`. Panel
membership *is* the surface's dock tree, so there is still no second list of
panels to keep in step — the same rule ADR 0002 set for the docked workspace.

`z` is the stacking position, written as the surface's index so restoring sorts
back into the same order. Bounds are plain `f32` in workspace coordinates rather
than `Pixels`, so geometry can be computed and tested without a window.

Schema 1 documents are still read: the field is additive and older files simply
have no surfaces. A *newer* schema is refused, because it may describe something
this build cannot reproduce. Restoring is all-or-nothing — if any surface fails
to load, the whole document is quarantined and the default preset is built, so a
half-restored workspace is never written back over the user's real one.

### 3. Moving a panel keeps the panel

A transfer between areas is:

1. mark the host handle as transferring (`HostPanel::begin_transfer`),
2. `add_panel_view` into the destination area,
3. `remove_panel` from the source area,
4. clear the mark.

The panel entity is never destroyed and never rebuilt from its dumped state, so
its scroll position, focus and plugin-internal state survive the move. Adding
before removing means the panel is never absent from every dock, even for an
instant.

Step 3 has a subtlety: a dock area that loses a panel reports it as *removed*,
which is the signal plugins tear down on (`Panel::on_removed`). A move is not a
teardown, so the host handle withholds that notice while the flag is set — and
clears the flag inside `on_removed` itself, so a transfer that fails half way
cannot leave the panel deaf to a real close later. The panel hears
`on_added_to` from its new group as usual.

Removing a panel from an area requires the concrete entity, and
`Arc<dyn PanelView>` cannot name it — that is the one place the type-erased dock
handle is not enough. `panels::PanelHandle` recovers the concrete entity through
`PanelView::as_any` for the two kinds the workspace creates (`HostPanel`, and the
`MissingPanel` placeholder). A view the workspace did not create has no handle
and is not moved; the command reports it rather than guessing.

### 4. Geometry lives in the surface view, stacking in the workspace

`FloatingSurfaceView` renders itself absolutely at its own bounds inside the
workspace's stacking container, and reports `Raised`, `BoundsChanged` and
`DockRequested` to the workspace. The workspace owns the *order* of surfaces
(and therefore stacking), persistence, and the panel transfers. The surface
knows nothing about the catalog, the panels it holds, or the workspace — which is
what lets the whole thing be exercised as one replaceable layer.

Moving and resizing use one code path: the title bar and the right, bottom and
corner grips start a GPUI drag carrying a marker value; `on_drag_move` reports
the pointer, and the new bounds are computed from the pointer and the grab point
recorded on mouse-down, then clamped into the viewport. There is no bespoke mouse
capture and no per-grip arithmetic beyond which axes the grip resizes.

Restored bounds are clamped into the current viewport, so a surface saved in a
larger window comes back fully visible instead of partly off the edge.

### 5. Commands first, cross-surface drag later

Because Base Dock's drag sessions are per-area and carry no area identity, a
drag that starts on a tab cannot be finished on another area without a
workspace-level drag coordinator that Base does not provide. Step 3 therefore
ships the transfer seam plus explicit commands, and defers drag-across-surfaces
to the polish pass:

- `ctrl-shift-f` — float the active panel of the area under the pointer; pressed
  again over a floating panel, it docks that panel back into the workspace.
- `ctrl-shift-d` — dock every floating panel back and close the surfaces.
- The surface's **Dock** button — dock that surface's panels back and close it.
- Clicking a surface raises it and gives it focus.

"Active panel" is the panel with focus in the area under the pointer, falling
back to the displayed tab of that area's first group. The pointer decides which
area; the fallback makes the command deterministic when nothing has focus.

Input on a floating surface is never swallowed: the whole surface raises on
mouse-down, so clicking a panel inside a floating group brings the group forward
as a side effect of the click the panel already wanted.

## Verification

Headless (Xvfb, `BRUCEBOX_DATA_DIR`, `scripts/x11-*.py`):

1. `ctrl-shift-f` on the default workspace → the Welcome panel leaves the docked
   area and appears in a new `Floating 1` surface; the document gains a surface
   with the panel's envelope (instance `1`, owner `brucebox.demo`).
2. Title-bar drag and corner-grip drag → bounds and size change on screen and
   are written to the document (`x 332, y 186, 775×521`).
3. Close the window 0.3 s after a resize → the surface and its final bounds are
   in the saved document (inside the 400 ms debounce, so this is the
   window-closed flush, not the debounced save).
4. Relaunch → the surface is restored at the same bounds with the same panel.
5. Float a second panel from the docked area → it joins the topmost surface as a
   tab; float it again with the pointer over the surface → it returns to the
   docked area and the surface keeps its remaining panel.
6. `ctrl-shift-d` → surfaces are gone and all three panels are back in the
   docked workspace.
7. Hand-edit the document to put an unregistered `brucebox.agent-chat.chat`
   panel inside a surface → it restores as a placeholder in that surface, and
   the panel's instance and payload survive the next save.
8. `ctrl-shift-r` with a surface open → default preset, no surfaces, document
   rewritten.
9. Two surfaces with different `z` restore with the right geometry and stacking.

## Known limits

- One new surface at a time: floating always targets the topmost existing
  surface, so the current UI never opens a second one. Restoring, raising and
  persisting N surfaces already work; opening more becomes meaningful with the
  launcher (step 8).
- No snapping, no multi-monitor, no per-surface maximise/roll-up — deferred by
  the plan.
- Surfaces are constrained to the window; a surface cannot be left partly
  outside it.

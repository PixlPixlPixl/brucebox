//! Floating surfaces: identity and geometry, as data.
//!
//! A floating surface is a group of panels that sits above the docked
//! workspace, can be moved and resized independently, and has its own stacking
//! order. It owns a GPUI Base Dock area of its own, so the panels inside it keep
//! the same tabs and splits as the main workspace.
//!
//! Everything here is pure: identity minting, bounds arithmetic, and the
//! persisted shape. Creating dock areas and moving panels between them belongs
//! to [`crate::workspace::dock`] and [`crate::workspace::floating`].

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui_kit::base::dock::DockAreaState;
use serde::{Deserialize, Serialize};

/// Smallest width a surface may be resized to.
pub const MIN_SURFACE_WIDTH: f32 = 240.;
/// Smallest height a surface may be resized to.
pub const MIN_SURFACE_HEIGHT: f32 = 160.;
/// Width a new surface starts at, before it is clamped to the workspace.
pub const DEFAULT_SURFACE_WIDTH: f32 = 620.;
/// Height a new surface starts at, before it is clamped to the workspace.
pub const DEFAULT_SURFACE_HEIGHT: f32 = 440.;

/// Distance between the origins of two cascaded surfaces, and from the
/// workspace origin.
const CASCADE_STEP: f32 = 32.;

static NEXT_SURFACE: AtomicU64 = AtomicU64::new(1);

/// The unique identity of one floating surface.
///
/// Kept across restarts so a persisted surface can be matched to the dock area
/// rebuilt for it, and so a later surface cannot adopt an id already on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceId(u64);

impl SurfaceId {
    /// Mint the next surface id.
    pub fn next() -> Self {
        Self(NEXT_SURFACE.fetch_add(1, Ordering::Relaxed))
    }

    /// Restore a persisted surface id, moving the mint counter past it.
    pub fn adopt(raw: u64) -> Self {
        NEXT_SURFACE.fetch_max(raw.saturating_add(1), Ordering::Relaxed);
        Self(raw)
    }

    /// The raw value, for persisted state.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for SurfaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The area a surface is laid out inside, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfaceArea {
    pub width: f32,
    pub height: f32,
}

impl SurfaceArea {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: width.max(0.),
            height: height.max(0.),
        }
    }
}

/// Where a surface sits, in workspace coordinates, and how large it is.
///
/// Stored as plain `f32` rather than GPUI's `Pixels` so the persisted document
/// stays independent of the UI layer's units, and so geometry can be reasoned
/// about (and tested) without a window.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SurfaceBounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl SurfaceBounds {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Where surface number `index` (0-based) opens: cascaded down and to the
    /// right, so a second surface does not land exactly on the first.
    pub fn cascaded(index: usize, area: SurfaceArea) -> Self {
        let offset = CASCADE_STEP * (index as f32 + 1.);
        Self::new(
            offset,
            offset,
            DEFAULT_SURFACE_WIDTH,
            DEFAULT_SURFACE_HEIGHT,
        )
        .clamped(area)
    }

    /// Fit these bounds inside `area`, keeping the minimum size.
    ///
    /// A surface that was saved in a larger window comes back fully visible
    /// rather than partly off the edge, and one that is dragged out of the
    /// workspace is pulled back in.
    pub fn clamped(self, area: SurfaceArea) -> Self {
        let width = sanitize(self.width)
            .max(MIN_SURFACE_WIDTH)
            .min(area.width.max(MIN_SURFACE_WIDTH));
        let height = sanitize(self.height)
            .max(MIN_SURFACE_HEIGHT)
            .min(area.height.max(MIN_SURFACE_HEIGHT));
        let x = sanitize(self.x).clamp(0., (area.width - width).max(0.));
        let y = sanitize(self.y).clamp(0., (area.height - height).max(0.));

        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// This surface translated by a pointer delta.
    pub fn moved(self, dx: f32, dy: f32) -> Self {
        Self {
            x: sanitize(self.x + dx),
            y: sanitize(self.y + dy),
            ..self
        }
    }

    /// This surface resized, keeping its origin.
    pub fn resized(self, width: f32, height: f32) -> Self {
        Self {
            width: sanitize(width),
            height: sanitize(height),
            ..self
        }
    }

    /// Whether `(x, y)` falls inside these bounds.
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Keep a coordinate finite, so a saved document cannot poison the layout.
fn sanitize(value: f32) -> f32 {
    if value.is_finite() { value } else { 0. }
}

/// A floating surface as it is written to the workspace document.
///
/// The panel membership lives inside `dock`: a surface's panels *are* the leaves
/// of its dock tree, so there is no second list to keep in step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloatingSurfaceState {
    id: u64,
    bounds: SurfaceBounds,
    z: u32,
    dock: DockAreaState,
}

impl FloatingSurfaceState {
    pub fn new(id: SurfaceId, bounds: SurfaceBounds, z: u32, dock: DockAreaState) -> Self {
        Self {
            id: id.get(),
            bounds,
            z,
            dock,
        }
    }

    /// The persisted surface id, readable before the id type is restored.
    pub fn raw_id(&self) -> u64 {
        self.id
    }

    pub fn id(&self) -> SurfaceId {
        SurfaceId::adopt(self.id)
    }

    pub fn bounds(&self) -> SurfaceBounds {
        self.bounds
    }

    pub fn z(&self) -> u32 {
        self.z
    }

    pub fn dock(&self) -> &DockAreaState {
        &self.dock
    }

    pub fn into_dock(self) -> DockAreaState {
        self.dock
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FloatingSurfaceState, MIN_SURFACE_HEIGHT, MIN_SURFACE_WIDTH, SurfaceArea, SurfaceBounds,
        SurfaceId,
    };
    use gpui_kit::base::dock::{DockAreaState, PanelState};

    fn dock() -> DockAreaState {
        DockAreaState {
            version: Some(1),
            center: PanelState::new("brucebox.demo.welcome"),
            left_dock: None,
            right_dock: None,
            bottom_dock: None,
        }
    }

    #[test]
    fn a_new_surface_keeps_its_minimum_size() {
        let area = SurfaceArea::new(1200., 900.);
        let bounds = SurfaceBounds::new(10., 20., 10., 10.).clamped(area);

        assert_eq!(bounds.width, MIN_SURFACE_WIDTH);
        assert_eq!(bounds.height, MIN_SURFACE_HEIGHT);
        assert_eq!(bounds.x, 10.);
        assert_eq!(bounds.y, 20.);
    }

    #[test]
    fn bounds_are_pulled_back_inside_the_workspace() {
        let area = SurfaceArea::new(800., 600.);

        let off_the_right = SurfaceBounds::new(5000., 5000., 300., 200.).clamped(area);
        assert_eq!(off_the_right.x, 500.);
        assert_eq!(off_the_right.y, 400.);

        let off_the_left = SurfaceBounds::new(-400., -300., 300., 200.).clamped(area);
        assert_eq!(off_the_left.x, 0.);
        assert_eq!(off_the_left.y, 0.);
    }

    #[test]
    fn a_surface_larger_than_the_workspace_is_shrunk_to_fit() {
        let area = SurfaceArea::new(500., 400.);
        let bounds = SurfaceBounds::new(0., 0., 4000., 3000.).clamped(area);

        assert_eq!(bounds.width, 500.);
        assert_eq!(bounds.height, 400.);
        assert_eq!(bounds.x, 0.);
        assert_eq!(bounds.y, 0.);
    }

    #[test]
    fn a_workspace_smaller_than_the_minimum_still_yields_a_usable_surface() {
        let area = SurfaceArea::new(120., 90.);
        let bounds = SurfaceBounds::new(0., 0., 400., 300.).clamped(area);

        // Not clamped below the minimum: a surface the user can still drag and
        // resize back is better than one squeezed to the window.
        assert_eq!(bounds.width, MIN_SURFACE_WIDTH);
        assert_eq!(bounds.height, MIN_SURFACE_HEIGHT);
    }

    #[test]
    fn cascaded_surfaces_do_not_stack_exactly() {
        let area = SurfaceArea::new(1200., 900.);
        let first = SurfaceBounds::cascaded(0, area);
        let second = SurfaceBounds::cascaded(1, area);

        assert_ne!((first.x, first.y), (second.x, second.y));
        assert!(second.x > first.x && second.y > first.y);
        assert_eq!(first.clamped(area), first);
        assert_eq!(second.clamped(area), second);
    }

    #[test]
    fn a_saved_coordinate_that_is_not_finite_does_not_poison_the_layout() {
        let area = SurfaceArea::new(800., 600.);
        let bounds = SurfaceBounds::new(f32::NAN, f32::INFINITY, f32::NAN, 300.).clamped(area);

        assert_eq!(bounds.x, 0.);
        assert_eq!(bounds.y, 0.);
        assert_eq!(bounds.width, MIN_SURFACE_WIDTH);
        assert_eq!(bounds.height, 300.);
    }

    #[test]
    fn bounds_move_and_resize_without_changing_the_other_axis() {
        let bounds = SurfaceBounds::new(100., 100., 400., 300.);

        assert_eq!(
            bounds.moved(20., -50.),
            SurfaceBounds::new(120., 50., 400., 300.)
        );
        assert_eq!(
            bounds.resized(500., 350.),
            SurfaceBounds::new(100., 100., 500., 350.)
        );
    }

    #[test]
    fn contains_is_half_open_so_neighbouring_surfaces_do_not_both_hit() {
        let bounds = SurfaceBounds::new(10., 10., 100., 100.);

        assert!(bounds.contains(10., 10.));
        assert!(bounds.contains(109.9, 109.9));
        assert!(!bounds.contains(110., 50.));
        assert!(!bounds.contains(5., 50.));
    }

    #[test]
    fn a_surface_state_round_trips_through_json() {
        let state = FloatingSurfaceState::new(
            SurfaceId::next(),
            SurfaceBounds::new(32., 32., 620., 440.),
            7,
            dock(),
        );

        let json = serde_json::to_string(&state).expect("surface state serializes");
        let restored: FloatingSurfaceState =
            serde_json::from_str(&json).expect("surface state parses");

        assert_eq!(restored, state);
        assert_eq!(restored.id(), state.id());
        assert_eq!(restored.dock(), state.dock());
    }

    #[test]
    fn an_adopted_surface_id_is_not_minted_again() {
        let high = SurfaceId::next().get() + 1_000;

        let adopted = SurfaceId::adopt(high);

        assert_eq!(adopted.get(), high);
        assert!(SurfaceId::next().get() > high);
    }
}

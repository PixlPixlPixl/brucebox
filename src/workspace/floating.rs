//! The live floating surface: chrome, movement, resizing, and its dock area.
//!
//! A surface is a view of its own, holding one GPUI Base Dock area. The docked
//! workspace places it in its own stacking layer; everything about the surface's
//! own box — where it is, how big it is, whether it is on top — is decided here
//! and reported to the workspace through [`FloatingSurfaceEvent`].
//!
//! The surface deliberately does not know about the workspace, the catalog, or
//! which panels it holds. Docking its panels back is a request, not an action it
//! performs.
//!
//! # Moving and resizing
//!
//! The title bar and the three grips start a GPUI drag carrying a
//! [`SurfaceDrag`] marker. While that drag is live, `on_drag_move` reports every
//! pointer position, and the bounds are recomputed from the pointer and the
//! grab point recorded on mouse-down. Dragging therefore never needs mouse
//! capture of its own, and the same code path serves moving and resizing.

use gpui_kit::base::dock::DockArea;
use gpui_kit::*;

use crate::workspace::surface::{SurfaceArea, SurfaceBounds, SurfaceId};

/// What a surface tells the workspace.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FloatingSurfaceEvent {
    /// The user interacted with this surface, so it should come to the front.
    Raised(SurfaceId),
    /// The user asked for this surface's panels to go back into the dock.
    DockRequested(SurfaceId),
    /// The surface moved or was resized, and the workspace should be saved.
    BoundsChanged(SurfaceId),
}

/// Which part of the surface is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceGrip {
    TitleBar,
    Right,
    Bottom,
    Corner,
}

/// A point and size in workspace coordinates, captured when a drag started.
#[derive(Debug, Clone, Copy)]
struct DragStart {
    grip: SurfaceGrip,
    pointer: Point<Pixels>,
    bounds: SurfaceBounds,
}

impl DragStart {
    /// The bounds this drag produces at `pointer`.
    fn bounds_at(&self, pointer: Point<Pixels>, area: SurfaceArea) -> SurfaceBounds {
        let dx = f32::from(pointer.x - self.pointer.x);
        let dy = f32::from(pointer.y - self.pointer.y);
        let origin = self.bounds;

        match self.grip {
            SurfaceGrip::TitleBar => origin.moved(dx, dy),
            SurfaceGrip::Right => origin.resized(origin.width + dx, origin.height),
            SurfaceGrip::Bottom => origin.resized(origin.width, origin.height + dy),
            SurfaceGrip::Corner => origin.resized(origin.width + dx, origin.height + dy),
        }
        .clamped(area)
    }
}

/// The value GPUI carries while a surface drag is in progress.
///
/// It marks *that* a surface is being dragged; the geometry lives in the view,
/// recorded on mouse-down, so the drag value itself stays a tiny `Copy` marker.
#[derive(Debug, Clone, Copy)]
struct SurfaceDrag;

/// The translucent rectangle drawn under the pointer during a drag.
struct SurfaceDragPreview {
    width: f32,
    height: f32,
    grip: SurfaceGrip,
}

impl Render for SurfaceDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // A full-size outline for a move, a small block for a resize grip:
        // the preview should show what the pointer is about to change.
        let (width, height) = match self.grip {
            SurfaceGrip::TitleBar => (self.width, self.height),
            _ => (24., 24.),
        };

        div()
            .w(px(width))
            .h(px(height))
            .bg(rgba(0x2563eb33))
            .border_1()
            .border_color(rgba(0x2563ebcc))
    }
}

pub struct FloatingSurfaceView {
    id: SurfaceId,
    title: SharedString,
    bounds: SurfaceBounds,
    dock_area: Entity<DockArea>,
    focus_handle: FocusHandle,
    drag: Option<DragStart>,
}

impl FloatingSurfaceView {
    /// Build a surface around an already-created dock area.
    pub fn new(
        id: SurfaceId,
        title: impl Into<SharedString>,
        bounds: SurfaceBounds,
        dock_area: Entity<DockArea>,
        cx: &mut App,
    ) -> Entity<Self> {
        cx.new(|cx| Self {
            id,
            title: title.into(),
            bounds,
            dock_area,
            focus_handle: cx.focus_handle(),
            drag: None,
        })
    }

    pub fn id(&self) -> SurfaceId {
        self.id
    }

    pub fn title(&self) -> &SharedString {
        &self.title
    }

    pub fn bounds(&self) -> SurfaceBounds {
        self.bounds
    }

    /// The dock area that holds this surface's panels.
    pub fn dock_area(&self) -> &Entity<DockArea> {
        &self.dock_area
    }

    /// Place the surface at `bounds`, telling the workspace to save.
    ///
    /// Used both by a drag and by the workspace after clamping a restored
    /// surface into a smaller window.
    pub fn set_bounds(&mut self, bounds: SurfaceBounds, cx: &mut Context<Self>) {
        if bounds == self.bounds {
            return;
        }
        self.bounds = bounds;
        cx.emit(FloatingSurfaceEvent::BoundsChanged(self.id));
        cx.notify();
    }

    fn start_drag(
        &mut self,
        grip: SurfaceGrip,
        pointer: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag = Some(DragStart {
            grip,
            pointer,
            bounds: self.bounds,
        });
        cx.emit(FloatingSurfaceEvent::Raised(self.id));
        self.focus_handle.focus(window, cx);
    }
}

impl EventEmitter<FloatingSurfaceEvent> for FloatingSurfaceView {}

impl Focusable for FloatingSurfaceView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for FloatingSurfaceView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bounds = self.bounds;

        div()
            .id(("brucebox-surface", self.id.get()))
            .absolute()
            .left(px(bounds.x))
            .top(px(bounds.y))
            .w(px(bounds.width))
            .h(px(bounds.height))
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(0x12161d))
            .border_1()
            .border_color(rgb(0x3b475a))
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    cx.emit(FloatingSurfaceEvent::Raised(this.id));
                }),
            )
            .child(self.render_title_bar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(self.dock_area.clone()),
            )
            .child(self.render_grip(SurfaceGrip::Right, cx))
            .child(self.render_grip(SurfaceGrip::Bottom, cx))
            .child(self.render_grip(SurfaceGrip::Corner, cx))
    }
}

impl FloatingSurfaceView {
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.title.clone();
        let id = self.id;

        div()
            .id(("brucebox-surface-title", id.get()))
            .h(px(28.))
            .flex_shrink_0()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px(px(8.))
            .bg(rgb(0x1d2530))
            .text_color(rgb(0xc7d0dd))
            .text_sm()
            .child(title)
            .child(
                div()
                    .id(("brucebox-surface-dock", id.get()))
                    .px(px(8.))
                    .py(px(2.))
                    .rounded_sm()
                    .bg(rgb(0x2b3546))
                    .child("Dock")
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.emit(FloatingSurfaceEvent::DockRequested(this.id));
                    })),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.start_drag(SurfaceGrip::TitleBar, event.position, window, cx);
                }),
            )
            .on_drag(SurfaceDrag, |_, _, _, cx| {
                cx.new(|_| SurfaceDragPreview {
                    width: 240.,
                    height: 160.,
                    grip: SurfaceGrip::TitleBar,
                })
            })
            .on_drag_move::<SurfaceDrag>(cx.listener(
                |this, event: &DragMoveEvent<SurfaceDrag>, window, cx| {
                    this.drag_to(event.event.position, window, cx);
                },
            ))
    }

    fn render_grip(&self, grip: SurfaceGrip, cx: &mut Context<Self>) -> impl IntoElement {
        // A right or bottom grip runs the full edge; the corner is a square
        // over both, so it is painted last and wins the pointer there.
        let (width, height) = match grip {
            SurfaceGrip::Right => (6., 6.),
            SurfaceGrip::Bottom => (6., 6.),
            _ => (14., 14.),
        };

        let mut grip_element = div()
            .id(("brucebox-surface-grip", self.id.get() * 4 + grip as u64))
            .absolute()
            .bg(rgba(0x3b475a88));

        grip_element = match grip {
            SurfaceGrip::Right => grip_element
                .top(px(28.))
                .right(px(0.))
                .w(px(width))
                .h(px(0.))
                .bottom(px(0.)),
            SurfaceGrip::Bottom => grip_element
                .left(px(0.))
                .bottom(px(0.))
                .h(px(height))
                .w(px(0.))
                .right(px(0.)),
            _ => grip_element
                .right(px(0.))
                .bottom(px(0.))
                .w(px(width))
                .h(px(height)),
        };

        grip_element
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.start_drag(grip, event.position, window, cx);
                }),
            )
            .on_drag(SurfaceDrag, move |_, _, _, cx| {
                cx.new(|_| SurfaceDragPreview {
                    width: 0.,
                    height: 0.,
                    grip,
                })
            })
            .on_drag_move::<SurfaceDrag>(cx.listener(
                |this, event: &DragMoveEvent<SurfaceDrag>, window, cx| {
                    this.drag_to(event.event.position, window, cx);
                },
            ))
    }

    /// Recompute the bounds from the live drag, if one is in progress.
    fn drag_to(&mut self, pointer: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else {
            return;
        };
        let viewport = window.viewport_size();
        let area = SurfaceArea::new(f32::from(viewport.width), f32::from(viewport.height));

        let bounds = drag.bounds_at(pointer, area);
        self.set_bounds(bounds, cx);
    }
}

//! Panel implementations and the host handle that wraps them.

use std::sync::Arc;

use gpui_kit::base::dock::{DockArea, PanelView};
use gpui_kit::*;

use self::host::HostPanel;
use self::missing::MissingPanel;

pub mod demo;
pub mod host;
pub mod missing;
pub mod plugins;

/// A panel view the workspace created, kept alongside the type-erased handle.
///
/// Moving a panel between the docked workspace and a floating surface means
/// taking it out of one [`DockArea`] and putting it into another. GPUI Base
/// names the panel by [`PanelId`] when it moves one inside a single area, but
/// removing it from an area goes through `DockArea::remove_panel`, which is
/// generic over the panel entity — and `Arc<dyn PanelView>` cannot name it.
/// The handle remembers the concrete entity, so a transfer can remove the panel
/// without tearing it down and rebuilding it from its dumped state.
///
/// A view the workspace did not create has no handle: the workspace never
/// makes those, and a panel it cannot remove is one it does not move.
///
/// [`PanelId`]: gpui_kit::base::dock::PanelId
pub(crate) enum PanelHandle {
    /// A catalog contribution, wrapped by the host.
    Host(Entity<HostPanel>),
    /// A placeholder standing in for a panel that could not be built.
    Missing(Entity<MissingPanel>),
}

impl PanelHandle {
    /// Recover the handle behind a dock panel view, if the workspace made it.
    pub(crate) fn of(view: &Arc<dyn PanelView>) -> Option<Self> {
        let any = view.as_any();

        if let Some(entity) = any.downcast_ref::<Entity<HostPanel>>() {
            return Some(Self::Host(entity.clone()));
        }
        if let Some(entity) = any.downcast_ref::<Entity<MissingPanel>>() {
            return Some(Self::Missing(entity.clone()));
        }
        None
    }

    /// Tell the panel it is being moved rather than closed.
    ///
    /// A dock area that loses a panel reports it as removed, which is the
    /// signal a plugin cleans up on. A transfer is not that: the panel keeps
    /// its entity and its state and lands in another area, so the removal
    /// notice is withheld.
    pub(crate) fn begin_transfer(&self, cx: &mut App) {
        if let Self::Host(entity) = self {
            entity.update(cx, |panel, _| panel.begin_transfer());
        }
    }

    /// Clear the transfer flag, for a move that did not go through.
    pub(crate) fn end_transfer(&self, cx: &mut App) {
        if let Self::Host(entity) = self {
            entity.update(cx, |panel, _| panel.end_transfer());
        }
    }

    /// Take the panel out of `area` without asking for a rebuild.
    pub(crate) fn remove_from(
        &self,
        area: &mut DockArea,
        window: &mut Window,
        cx: &mut Context<DockArea>,
    ) {
        match self {
            Self::Host(entity) => area.remove_panel(entity.clone(), window, cx),
            Self::Missing(entity) => area.remove_panel(entity.clone(), window, cx),
        }
    }
}

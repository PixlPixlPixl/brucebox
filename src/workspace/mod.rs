//! The workspace engine: panel catalog, layouts, docking, and presentation.
//!
//! Responsibilities are split so each piece can be reasoned about alone:
//!
//! - [`ids`] — plugin, panel type, and panel instance identity.
//! - [`catalog`] — what panels exist, who owns them, and how to create one.
//! - [`layout`] — layout data and the plan that resolves type ids to panels.
//! - [`preset`] — the first-run arrangement, as data.
//! - [`dock`] — the live GPUI Base Dock area.
//! - [`panels`] — the host handle plus the panels this build ships.
//! - [`presentation`] — the replaceable dock renderer.
//!
//! Nothing here knows a concrete plugin implementation. Later milestones add
//! persistence, floating groups, and the Cordis runtime on top of these seams.

pub mod catalog;
pub mod contributions;
pub mod dock;
pub mod ids;
pub mod layout;
pub mod panels;
pub mod presentation;
pub mod preset;

pub use dock::BruceboxWorkspace;

use gpui_kit::App;

/// Install the workspace-level host services and startup contributions.
///
/// Must run after GPUI Kit Base is initialised and before a window is opened:
/// the dock's restore registry and the panel catalog both have to exist before
/// the first layout is resolved.
pub fn init(cx: &mut App) -> Result<(), catalog::CatalogError> {
    catalog::init(cx);
    contributions::register(cx)
}

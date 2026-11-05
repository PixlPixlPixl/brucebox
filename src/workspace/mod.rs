//! The workspace engine: panel catalog, layouts, docking, and persistence.
//!
//! Responsibilities are split so each piece can be reasoned about alone:
//!
//! - [`ids`] — plugin, panel type, and panel instance identity.
//! - [`catalog`] — what panels exist, who owns them, and how to create one.
//! - [`layout`] — layout data and the plan that resolves type ids to panels.
//! - [`preset`] — the first-run arrangement, as data.
//! - [`document`] — the versioned document a workspace is saved as.
//! - [`store`] — where that document lives and how it is written.
//! - [`dock`] — the live GPUI Base Dock area and its save lifecycle.
//! - [`panels`] — the host handle plus the panels this build ships.
//! - [`presentation`] — the replaceable dock renderer.
//!
//! Nothing here knows a concrete plugin implementation. Later milestones add
//! floating groups and the Cordis runtime on top of these seams.

pub mod catalog;
pub mod contributions;
pub mod dock;
pub mod document;
pub mod ids;
pub mod layout;
pub mod panels;
pub mod presentation;
pub mod preset;
pub mod store;

pub use dock::BruceboxWorkspace;

use gpui_kit::App;

gpui_kit::actions!(brucebox, [ResetWorkspace]);

/// Key binding for the explicit "reset this workspace to the default preset".
pub const RESET_WORKSPACE_KEY: &str = "ctrl-shift-r";

/// Install the workspace-level host services, startup contributions, and input
/// bindings.
///
/// Must run after GPUI Kit Base is initialised and before a window is opened:
/// the dock's restore registry and the panel catalog both have to exist before
/// the first layout is resolved.
pub fn init(cx: &mut App) -> Result<(), catalog::CatalogError> {
    catalog::init(cx);
    contributions::register(cx)?;

    cx.bind_keys([gpui_kit::KeyBinding::new(
        RESET_WORKSPACE_KEY,
        ResetWorkspace,
        None,
    )]);

    Ok(())
}

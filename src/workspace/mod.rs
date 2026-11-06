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
//! - [`surface`] — floating surface identity, geometry, and persisted shape.
//! - [`floating`] — the live floating surface view, its chrome, and its drags.
//! - [`dock`] — the live workspace: docked areas, floating surfaces, saves.
//! - [`panels`] — the host handle plus the panels this build ships.
//! - [`presentation`] — the replaceable dock renderer.
//!
//! Nothing here knows a concrete plugin implementation. The Cordis runtime and
//! the plugin loader arrive in later milestones on top of these seams.

pub mod catalog;
pub mod contributions;
pub mod dock;
pub mod document;
pub mod floating;
pub mod ids;
pub mod layout;
pub mod panels;
pub mod presentation;
pub mod preset;
pub mod store;
pub mod surface;

pub use dock::BruceboxWorkspace;

use gpui_kit::App;

gpui_kit::actions!(
    brucebox,
    [ResetWorkspace, FloatActivePanel, DockFloatingSurfaces]
);

/// Key binding for the explicit "reset this workspace to the default preset".
pub const RESET_WORKSPACE_KEY: &str = "ctrl-shift-r";
/// Key binding for "float the active panel, or dock it again".
pub const FLOAT_ACTIVE_PANEL_KEY: &str = "ctrl-shift-f";
/// Key binding for "bring every floating panel back into the docked workspace".
pub const DOCK_FLOATING_SURFACES_KEY: &str = "ctrl-shift-d";

/// Install the workspace-level host services, startup contributions, and input
/// bindings.
///
/// Must run after GPUI Kit Base is initialised and before a window is opened:
/// the dock's restore registry and the panel catalog both have to exist before
/// the first layout is resolved.
pub fn init(cx: &mut App) -> Result<(), catalog::CatalogError> {
    catalog::init(cx);
    contributions::register(cx)?;

    cx.bind_keys([
        gpui_kit::KeyBinding::new(RESET_WORKSPACE_KEY, ResetWorkspace, None),
        gpui_kit::KeyBinding::new(FLOAT_ACTIVE_PANEL_KEY, FloatActivePanel, None),
        gpui_kit::KeyBinding::new(DOCK_FLOATING_SURFACES_KEY, DockFloatingSurfaces, None),
    ]);

    Ok(())
}

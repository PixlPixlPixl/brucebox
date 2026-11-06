//! The workspace engine: panel catalog, layouts, docking, and persistence.
//!
//! Responsibilities are split so each piece can be reasoned about alone:
//!
//! - [`ids`] — plugin, panel type, and panel instance identity.
//! - [`catalog`] — what panels exist, who owns them, and how to create one.
//! - [`declaration`] — what a plugin says it contributes, as data.
//! - [`layout`] — layout data and the plan that resolves type ids to panels.
//! - [`preset`] — the first-run arrangement, as data.
//! - [`document`] — the versioned document a workspace is saved as.
//! - [`store`] — where that document lives and how it is written.
//! - [`surface`] — floating surface identity, geometry, and persisted shape.
//! - [`floating`] — the live floating surface view, its chrome, and its drags.
//! - [`dock`] — the live workspace: docked areas, floating surfaces, saves.
//! - [`panels`] — the host handle plus the panels this build ships.
//! - [`status`] — what the host's composition runtime is doing.
//! - [`presentation`] — the replaceable dock renderer.
//!
//! Nothing here knows a concrete plugin implementation, and nothing here imports
//! the composition runtime: a plugin crosses into the workspace as a
//! [`declaration`], and a lifecycle fact crosses as [`status`] data. The Cordis
//! runtime (`crate::runtime`) sits on top of both.

pub mod catalog;
pub mod contributions;
pub mod declaration;
pub mod dock;
pub mod document;
pub mod floating;
pub mod ids;
pub mod layout;
pub mod panels;
pub mod presentation;
pub mod preset;
pub mod status;
pub mod store;
pub mod surface;

pub use dock::BruceboxWorkspace;

use gpui_kit::App;

gpui_kit::actions!(
    brucebox,
    [
        ResetWorkspace,
        FloatActivePanel,
        DockFloatingSurfaces,
        OpenPanelCatalog
    ]
);

/// Key binding for the explicit "reset this workspace to the default preset".
pub const RESET_WORKSPACE_KEY: &str = "ctrl-shift-r";
/// Key binding for "float the active panel, or dock it again".
pub const FLOAT_ACTIVE_PANEL_KEY: &str = "ctrl-shift-f";
/// Key binding for "bring every floating panel back into the docked workspace".
pub const DOCK_FLOATING_SURFACES_KEY: &str = "ctrl-shift-d";
/// Key binding for "open the panel catalog".
pub const OPEN_PANEL_CATALOG_KEY: &str = "ctrl-shift-p";

/// The workspace that is open, for panels that ask the host to open another one.
///
/// The panel catalog is a normal panel: it can be closed, and this is what lets
/// it (or any future panel) ask the workspace to open a contribution without
/// knowing the dock's type. It is a host-side registry, not a plugin API.
pub struct ActiveWorkspace(pub gpui_kit::Entity<dock::BruceboxWorkspace>);

impl gpui_kit::Global for ActiveWorkspace {}

/// Remember the workspace this window shows.
pub fn install_active_workspace(
    cx: &mut App,
    workspace: gpui_kit::Entity<dock::BruceboxWorkspace>,
) {
    cx.set_global(ActiveWorkspace(workspace));
}

/// Open one registered panel type in the active workspace.
///
/// Fails when no workspace is open, when nothing registers the type, or when the
/// type already has as many instances open as it allows.
pub fn open_panel(
    window: &mut gpui_kit::Window,
    cx: &mut App,
    panel_type: &ids::PanelTypeId,
) -> Result<(), String> {
    let Some(workspace) = cx
        .try_global::<ActiveWorkspace>()
        .map(|active| active.0.clone())
    else {
        return Err("no workspace is open".to_owned());
    };
    workspace.update(cx, |workspace, cx| {
        workspace.open_panel(panel_type, window, cx)
    })
}

/// Install the workspace-level host services, panel factories, and input
/// bindings.
///
/// Must run after GPUI Kit Base is initialised and before a window is opened:
/// the dock's restore registry and the panel catalog both have to exist before
/// the first layout is resolved, and the factories have to be in place before
/// the composition runtime declares its contributions — which happens in
/// [`runtime::install`](crate::runtime::install), before the window opens too.
pub fn init(cx: &mut App) -> Result<(), catalog::CatalogError> {
    catalog::init(cx);
    status::init(cx);

    // Only the factories are installed here. Which panels exist is decided by
    // the composition runtime: it submits the contributions this build ships as
    // a plugin, and those arrive as declarations. Registering the same panels
    // here as well would make the declaration a duplicate.
    contributions::install_factories(cx);

    cx.bind_keys([
        gpui_kit::KeyBinding::new(RESET_WORKSPACE_KEY, ResetWorkspace, None),
        gpui_kit::KeyBinding::new(FLOAT_ACTIVE_PANEL_KEY, FloatActivePanel, None),
        gpui_kit::KeyBinding::new(DOCK_FLOATING_SURFACES_KEY, DockFloatingSurfaces, None),
        gpui_kit::KeyBinding::new(OPEN_PANEL_CATALOG_KEY, OpenPanelCatalog, None),
    ]);

    Ok(())
}

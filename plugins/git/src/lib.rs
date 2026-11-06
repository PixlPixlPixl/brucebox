//! Git — a first-party Brucebox plugin.
//!
//! A repository panel, a diff panel, and a published contract: the plugin re-reads
//! the repository, publishes a snapshot and the selected diff on the host bus, and
//! any other plugin may consume them. It is built as a standalone `cdylib` against
//! `brucebox-sdk`; nothing here is compiled into the host.
//!
//! Two panels over one store, so a workspace can dock, split, tab or float them
//! independently, or open no Diff panel at all.

mod panels;
mod repository;
mod store;
mod watch;

use brucebox_sdk::prelude::*;

/// This plugin's stable id.
pub const PLUGIN_ID: &str = "brucebox.git";

/// Builds one repository panel.
fn create_repository(open: &PanelOpen<'_>, _window: &mut Window, cx: &mut App) -> PanelParts {
    let panel = panels::RepositoryPanel::new(open, cx);
    interactive_panel(panel, panels::RepositoryPanel::on_input)
}

/// Builds one diff panel.
fn create_diff(open: &PanelOpen<'_>, _window: &mut Window, cx: &mut App) -> PanelParts {
    panel_view(panels::DiffPanel::new(open, cx))
}

declare_plugin!(
    Plugin::new(PLUGIN_ID, "Git", env!("CARGO_PKG_VERSION"))
        .panel("repository", "Repository", create_repository)
        .multi_instance_panel("diff", "Diff", create_diff)
        .runtime(watch::prepare)
);

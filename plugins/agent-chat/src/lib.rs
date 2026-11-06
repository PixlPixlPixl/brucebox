//! Agent Chat — a first-party Brucebox plugin.
//!
//! A conversation panel plus a small composition contribution. It is built as a
//! standalone `cdylib` against `brucebox-sdk` and loaded by the host at runtime:
//! nothing here is compiled into Brucebox, and adding it to an installation is
//! copying a packaged folder into a plugin root and restarting.

mod chat;
mod session;

use brucebox_sdk::prelude::*;

/// This plugin's stable id.
pub const PLUGIN_ID: &str = "brucebox.agent-chat";

/// Builds one conversation panel for the host.
///
/// The panel is interactive: the host forwards this panel's keys as plain data
/// (`interactive_panel`), because GPUI's own listeners cannot fire across a
/// native library boundary.
fn create_chat(open: &PanelOpen<'_>, _window: &mut Window, cx: &mut App) -> PanelParts {
    let panel = chat::ChatPanel::new(open, cx);
    interactive_panel(panel, chat::ChatPanel::on_input)
}

declare_plugin!(
    Plugin::new(PLUGIN_ID, "Agent Chat", env!("CARGO_PKG_VERSION"))
        .panel("chat", "Chat", create_chat)
        .runtime(session::prepare)
);

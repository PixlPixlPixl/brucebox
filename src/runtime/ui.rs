//! The application-thread end of the runtime bridge.
//!
//! Every message that reaches the interface is applied here, on the GPUI thread,
//! with a real `&mut App`: the catalog accepts or refuses a contribution, the
//! host status records a phase, and the views that observe that status
//! re-render. Nothing in this module is reachable from a background thread.

use gpui_kit::{App, AsyncApp, Task};

use crate::workspace::declaration::PluginDeclaration;
use crate::workspace::{catalog, status};

use super::bridge::{UiInbox, UiMessage};

/// Apply one runtime message.
pub fn apply(cx: &mut App, message: UiMessage) {
    match message {
        UiMessage::Declared(declaration) => declare(cx, &declaration),
        UiMessage::Withdrawn(plugin) => match catalog::withdraw_declaration(cx, &plugin) {
            Ok(removed) => status::update(cx, |status| {
                status.note(format!("{plugin} withdrew {} panel type(s)", removed.len()));
            }),
            Err(error) => status::update(cx, |status| {
                status.note(format!("{plugin} could not be withdrawn: {error}"));
            }),
        },
        UiMessage::Phase {
            plugin,
            phase,
            detail,
        } => status::update(cx, |status| status.set_phase(plugin, phase, detail)),
        UiMessage::Note(note) => status::update(cx, |status| status.note(note)),
    }
}

/// Accept one plugin's contributions, recording why when the host refuses.
fn declare(cx: &mut App, declaration: &PluginDeclaration) {
    let plugin = declaration.plugin().as_str().to_owned();
    match catalog::register_declaration(cx, declaration) {
        Ok(()) => status::update(cx, |status| {
            status.note(format!(
                "{plugin} contributed {} panel type(s)",
                declaration.panels().len()
            ));
        }),
        Err(error) => status::update(cx, |status| {
            status.note(format!("{plugin} was not accepted: {error}"));
        }),
    }
}

/// Spawn the task that drains the mailbox for the rest of the application's
/// life.
///
/// The returned [`Task`] must be kept alive: dropping it cancels the drain, and
/// the interface would silently stop hearing from the runtime. It awaits a
/// message, applies it under the app lock, and asks for a redraw — the message
/// is the only thing that wakes this task, so an idle runtime costs nothing.
pub fn install_drain(cx: &mut App, inbox: UiInbox) -> Task<()> {
    cx.spawn(async move |cx: &mut AsyncApp| {
        let mut inbox = inbox;
        while let Some(message) = inbox.recv().await {
            cx.update(|cx| apply(cx, message));
            cx.refresh();
        }
    })
}

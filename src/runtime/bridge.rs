//! The mailbox between the composition runtime and the application thread.
//!
//! GPUI entity and window state belongs to the application thread, and plugin
//! apply bodies run wherever the composition runtime polls them (SPEC §6). The
//! two therefore never share a `&mut App`: the runtime *sends* a message and the
//! application thread decides what that means for the catalog and the views.
//!
//! Every message is plain data. Nothing here names a Cordis type, so the
//! workspace can apply a message without knowing a composition runtime exists.

use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::workspace::declaration::PluginDeclaration;
use crate::workspace::ids::PluginId;
use crate::workspace::status::PluginPhase;

/// One message from the composition runtime to the application thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiMessage {
    /// A plugin published the contributions the workspace should accept.
    Declared(PluginDeclaration),
    /// A plugin is unloading: drop its contributions.
    Withdrawn(PluginId),
    /// A plugin's lifecycle phase changed.
    Phase {
        /// Plugin id.
        plugin: String,
        /// The phase the runtime settled on.
        phase: PluginPhase,
        /// Why, when the runtime knows.
        detail: Option<String>,
    },
    /// A line for the host's status view.
    Note(String),
}

/// The sending half, cloned to everything that reports to the interface.
///
/// Sending never blocks and never needs a runtime: the channel is unbounded and
/// the application thread drains it whenever it next runs.
#[derive(Clone)]
pub struct UiMailbox {
    sender: UnboundedSender<UiMessage>,
}

impl UiMailbox {
    /// Send one message. Returns `false` once the application side is gone.
    ///
    /// A late message from a plugin that outlived the interface is not an
    /// error: dropping it is the correct end of that conversation.
    pub fn send(&self, message: UiMessage) -> bool {
        self.sender.send(message).is_ok()
    }

    /// Report a plugin's phase.
    pub fn phase(&self, plugin: &str, phase: PluginPhase, detail: Option<String>) -> bool {
        self.send(UiMessage::Phase {
            plugin: plugin.to_owned(),
            phase,
            detail,
        })
    }

    /// Add a line to the host status view.
    pub fn note(&self, note: impl Into<String>) -> bool {
        self.send(UiMessage::Note(note.into()))
    }
}

impl std::fmt::Debug for UiMailbox {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("UiMailbox")
    }
}

/// The receiving half, owned by the application thread.
pub struct UiInbox {
    receiver: UnboundedReceiver<UiMessage>,
}

impl UiInbox {
    /// Create the pair: a mailbox for the runtime, an inbox for the interface.
    pub fn channel() -> (UiMailbox, Self) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (UiMailbox { sender }, Self { receiver })
    }

    /// Every message that is ready right now, without waiting.
    ///
    /// Startup uses this to apply the first plugins' contributions before the
    /// first window resolves a layout, so the initial frame never depends on a
    /// later task being polled.
    pub fn drain(&mut self) -> Vec<UiMessage> {
        let mut drained = Vec::new();
        while let Ok(message) = self.receiver.try_recv() {
            drained.push(message);
        }
        drained
    }

    /// Wait for the next message. `None` once every sender is gone.
    pub async fn recv(&mut self) -> Option<UiMessage> {
        self.receiver.recv().await
    }

    /// The receiver, for a task that owns the draining loop.
    pub fn into_receiver(self) -> UnboundedReceiver<UiMessage> {
        self.receiver
    }
}

impl std::fmt::Debug for UiInbox {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("UiInbox")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::declaration::PanelDeclaration;
    use crate::workspace::ids::PanelTypeId;

    fn declaration() -> PluginDeclaration {
        let plugin = PluginId::parse("brucebox.git").unwrap();
        PluginDeclaration::new(plugin.clone(), "Git").with_panel(PanelDeclaration::new(
            PanelTypeId::new(&plugin, "status").unwrap(),
            "Status",
        ))
    }

    #[test]
    fn messages_arrive_in_order_and_drain_without_waiting() {
        let (mailbox, mut inbox) = UiInbox::channel();
        assert!(mailbox.phase("brucebox.git", PluginPhase::Pending, None));
        assert!(mailbox.send(UiMessage::Declared(declaration())));
        assert!(mailbox.note("two plugins admitted"));

        let messages = inbox.drain();
        assert_eq!(messages.len(), 3);
        assert!(matches!(messages[0], UiMessage::Phase { .. }));
        assert!(matches!(messages[1], UiMessage::Declared(_)));
        assert!(matches!(messages[2], UiMessage::Note(_)));
        assert!(
            inbox.drain().is_empty(),
            "draining twice yields nothing new"
        );
    }

    #[test]
    fn a_message_sent_after_the_interface_is_gone_is_dropped() {
        let (mailbox, inbox) = UiInbox::channel();
        drop(inbox);

        assert!(!mailbox.note("nobody is listening"));
        assert!(!mailbox.phase("brucebox.git", PluginPhase::Stopped, None));
    }
}

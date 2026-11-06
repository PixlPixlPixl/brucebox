//! The typed contracts the host owns.
//!
//! A contract is the only thing two independently written plugins share: a
//! service name plus its type, or an event name plus its typed arguments. The
//! host publishes very little of its own — the workspace service below and three
//! events — deliberately, because plugin-specific contracts belong to the
//! plugins that define them (SPEC §5.8), not to a central capability list.
//!
//! Nothing here touches GPUI. The workspace service is a *message* channel: it
//! turns plugin intent into [`UiMessage`]s and lets the application thread do
//! the work that needs a window.

use cordis::{Event, Service};

use crate::workspace::declaration::PluginDeclaration;
use crate::workspace::ids::PluginId;
use crate::workspace::status::PluginPhase;

use super::bridge::{UiMailbox, UiMessage};

/// The service name the workspace host publishes.
pub const WORKSPACE_SERVICE: &str = "brucebox.workspace";

/// The workspace, as a plugin sees it.
///
/// A plugin gets this from its apply context rather than constructing one, which
/// is what makes the runtime the composition system: the service exists only
/// while the plugin that published it is active, and dependants converge when it
/// appears or disappears.
pub struct WorkspaceService {
    mailbox: UiMailbox,
}

impl WorkspaceService {
    /// Wrap the host's mailbox.
    pub fn new(mailbox: UiMailbox) -> Self {
        Self { mailbox }
    }

    /// Publish panel contributions to the workspace.
    pub fn declare(&self, declaration: PluginDeclaration) -> bool {
        self.mailbox.send(UiMessage::Declared(declaration))
    }

    /// Withdraw everything a plugin contributed, without ending its own work.
    pub fn withdraw(&self, plugin: &PluginId) -> bool {
        self.mailbox.send(UiMessage::Withdrawn(plugin.clone()))
    }

    /// Report a plugin's lifecycle phase for the host status view.
    pub fn report(&self, plugin: &str, phase: PluginPhase, detail: Option<String>) -> bool {
        self.mailbox.phase(plugin, phase, detail)
    }

    /// Add a line to the host status view.
    pub fn note(&self, note: impl Into<String>) -> bool {
        self.mailbox.note(note)
    }

    /// The mailbox behind this service, for a task that needs to report
    /// asynchronously.
    pub fn mailbox(&self) -> &UiMailbox {
        &self.mailbox
    }
}

impl Service for WorkspaceService {
    const NAME: &'static str = WORKSPACE_SERVICE;
}

/// A plugin declared its panel contributions.
///
/// Declaring is an event rather than a service call so a plugin never has to
/// hold the workspace to contribute: publishing is enough, and any other plugin
/// that cares about contributions can observe the same event.
pub struct PanelsDeclared;

impl Event for PanelsDeclared {
    const NAME: &'static str = "brucebox.panels.declared";
    type Args = PluginDeclaration;
    type Output = ();
}

/// A plugin withdrew its panel contributions as it unloaded.
pub struct PanelsWithdrawn;

impl Event for PanelsWithdrawn {
    const NAME: &'static str = "brucebox.panels.withdrawn";
    type Args = PluginId;
    type Output = ();
}

/// The workspace service is available, so a dependant plugin may contribute.
pub struct WorkspaceReady;

impl Event for WorkspaceReady {
    const NAME: &'static str = "brucebox.workspace.ready";
    type Args = ();
    type Output = ();
}

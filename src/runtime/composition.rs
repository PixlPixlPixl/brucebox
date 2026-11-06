//! The host's own Cordis plugins.
//!
//! The host composes itself through the same runtime it gives plugins, which is
//! the point of having one: workspace services and events are not a special case
//! with a private code path.
//!
//! Two plugins make up the host:
//!
//! - [`WorkspaceAdapter`] publishes the workspace service and turns plugin
//!   events into messages for the application thread.
//! - [`ProvingContributions`] contributes this build's panels. It *requires* the
//!   workspace service, so the two demonstrate the two hard parts of composition
//!   in one pair: a dependency that must be present before apply runs, and typed
//!   events carrying a contribution across the plugin boundary.

use std::borrow::Cow;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use cordis::event::observer_sync;
use cordis::{Context, InjectSpec, Plugin, PreparedPlugin, Routing};

use crate::workspace::declaration::PluginDeclaration;
use crate::workspace::status::PluginPhase;

use super::bridge::{UiMailbox, UiMessage};
use super::contract::{
    PanelsDeclared, PanelsWithdrawn, WORKSPACE_SERVICE, WorkspaceReady, WorkspaceService,
};
use super::error::CompositionError;

/// Name the host gives its adapter plugin.
pub const WORKSPACE_PLUGIN: &str = "brucebox.host.workspace";

/// Name the host gives this build's built-in contributions.
pub const PROVING_PLUGIN: &str = "brucebox.demo";

/// How often [`ProvingContributions`] reports that its task is alive.
///
/// A heartbeat is the smallest thing that proves async work and the bridge are
/// both running: it stops when the generation closes.
pub const HEARTBEAT: Duration = Duration::from_secs(5);

/// The host adapter: publishes the workspace service, forwards plugin events.
pub struct WorkspaceAdapter {
    mailbox: UiMailbox,
}

impl WorkspaceAdapter {
    /// Seal the adapter for admission.
    pub fn prepared(mailbox: UiMailbox) -> PreparedPlugin {
        let plugin = Self {
            mailbox: mailbox.clone(),
        };
        PreparedPlugin::from_input(plugin, mailbox)
    }
}

impl std::fmt::Debug for WorkspaceAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceAdapter")
            .field("plugin", &WORKSPACE_PLUGIN)
            .finish()
    }
}

impl Plugin for WorkspaceAdapter {
    type Config = ();
    type Input = UiMailbox;
    type PrepareError = Infallible;
    type ApplyError = CompositionError;

    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(WORKSPACE_PLUGIN)
    }

    fn prepare(&self, (): ()) -> Result<Self::Input, Self::PrepareError> {
        Ok(self.mailbox.clone())
    }

    async fn apply(&self, ctx: Context, mailbox: &Self::Input) -> Result<(), Self::ApplyError> {
        // Published into this generation: plugins that require it stay Pending
        // until it exists, and converge when it disappears.
        let service = Arc::new(WorkspaceService::new(mailbox.clone()));
        let _publication = ctx
            .provide::<WorkspaceService>(service.clone())
            .map_err(|error| CompositionError::Service(error.to_string()))?;

        // Plugin → interface, one listener per event. Observer callbacks are
        // synchronous and only enqueue, so no plugin work runs on the
        // application thread.
        let declared = mailbox.clone();
        let _declared = ctx
            .on::<PanelsDeclared, _>(observer_sync(move |_, declaration| {
                declared.send(UiMessage::Declared(declaration));
                Ok::<(), Infallible>(())
            }))
            .map_err(|error| CompositionError::Listener(error.to_string()))?;

        let withdrawn = mailbox.clone();
        let _withdrawn = ctx
            .on::<PanelsWithdrawn, _>(observer_sync(move |_, plugin| {
                withdrawn.send(UiMessage::Withdrawn(plugin));
                Ok::<(), Infallible>(())
            }))
            .map_err(|error| CompositionError::Listener(error.to_string()))?;

        // Generation-owned cleanup: if this plugin ever unloads, the interface
        // must stop believing the service is there.
        let stopped = mailbox.clone();
        ctx.effect_sync(move || {
            stopped.send(UiMessage::Phase {
                plugin: WORKSPACE_PLUGIN.to_owned(),
                phase: PluginPhase::Stopped,
                detail: None,
            });
        })
        .map_err(|error| CompositionError::Effect(error.to_string()))?;

        ctx.emit::<WorkspaceReady>(Routing::Unscoped, ())
            .await
            .map_err(|error| CompositionError::Event(error.to_string()))?;

        service.note(format!("{WORKSPACE_PLUGIN} published {WORKSPACE_SERVICE}"));
        Ok(())
    }
}

/// This build's built-in panel contributions, submitted like any other plugin.
pub struct ProvingContributions {
    declaration: PluginDeclaration,
}

impl ProvingContributions {
    /// Seal the contributions for admission.
    pub fn prepared(declaration: PluginDeclaration) -> PreparedPlugin {
        let plugin = Self {
            declaration: declaration.clone(),
        };
        PreparedPlugin::from_input(plugin, declaration)
    }
}

impl std::fmt::Debug for ProvingContributions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProvingContributions")
            .field("plugin", &PROVING_PLUGIN)
            .field("panels", &self.declaration.panels().len())
            .finish()
    }
}

impl Plugin for ProvingContributions {
    type Config = ();
    type Input = PluginDeclaration;
    type PrepareError = Infallible;
    type ApplyError = CompositionError;

    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(PROVING_PLUGIN)
    }

    /// The workspace service is a lifecycle prerequisite, not a constructor
    /// argument: without it this plugin stays Pending and never runs apply.
    fn inject(&self) -> InjectSpec {
        InjectSpec::none().require(WORKSPACE_SERVICE)
    }

    fn prepare(&self, (): ()) -> Result<Self::Input, Self::PrepareError> {
        Ok(self.declaration.clone())
    }

    async fn apply(&self, ctx: Context, declaration: &Self::Input) -> Result<(), Self::ApplyError> {
        let service = ctx
            .try_service::<WorkspaceService>()
            .map_err(|error| CompositionError::Service(error.to_string()))?;

        // Publishing the declaration as a typed event is enough: this plugin
        // never holds the workspace, so the same event can be observed by
        // anything else that cares about contributions.
        ctx.emit::<PanelsDeclared>(Routing::Unscoped, declaration.clone())
            .await
            .map_err(|error| CompositionError::Event(error.to_string()))?;

        // Withdraw on generation close: the interface drops the panels this
        // plugin contributed, and a restored layout names panels that are no
        // longer there rather than panels nothing can build.
        let plugin_id = declaration.plugin().clone();
        let teardown = service.clone();
        ctx.effect_sync(move || {
            teardown.withdraw(&plugin_id);
            teardown.note(format!("{PROVING_PLUGIN} withdrew its contributions"));
        })
        .map_err(|error| CompositionError::Effect(error.to_string()))?;

        // Generation-owned async work. The task must observe its own teardown
        // and return, or the generation drain would wait on it forever: the
        // stop effect below is registered after the task, so LIFO runs it
        // first, and only then does the drain join.
        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let heartbeat = service.clone();
        ctx.run(async move {
            let mut tick: u64 = 0;
            loop {
                heartbeat.note(format!("{PROVING_PLUGIN} alive · tick {tick}"));
                tick += 1;
                tokio::select! {
                    () = tokio::time::sleep(HEARTBEAT) => {}
                    _ = stopped.changed() => break,
                }
            }
        })
        .map_err(|error| CompositionError::Effect(error.to_string()))?;

        ctx.effect_sync(move || {
            let _ = stop.send(true);
        })
        .map_err(|error| CompositionError::Effect(error.to_string()))?;

        service.note(format!(
            "{PROVING_PLUGIN} declared {} panel(s)",
            declaration.panels().len()
        ));
        Ok(())
    }
}

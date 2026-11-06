//! The host-owned Cordis composition runtime.
//!
//! There is one composition runtime for the whole application (SPEC §3.B): plugin
//! lifecycle, service dependencies, typed events and generation-owned cleanup all
//! live here, so two independently written plugins can share a service or hear
//! each other's events. Everything else in the host — the workspace, the catalog,
//! the panels — stays unaware of it:
//!
//! ```text
//!   plugin apply (runtime threads)            application thread
//!   ─────────────────────────────            ──────────────────
//!   WorkspaceService ─┐
//!   PanelsDeclared  ──┼─ UiMailbox ──────►── drain ──► catalog + HostStatus
//!   phase reports   ──┘   (plain data)                 (GPUI entities)
//! ```
//!
//! Three decisions carry that:
//!
//! 1. **The boundary is messages, not calls.** A plugin never touches a GPUI
//!    entity or window; it sends [`UiMessage`]s that the application thread
//!    applies with its own context (SPEC §6).
//! 2. **Contributions are declared, not built.** A panel crosses the boundary as
//!    [`PluginDeclaration`](crate::workspace::declaration::PluginDeclaration)
//!    data; the factory that builds it stays on the application thread, because
//!    it produces GPUI entities.
//! 3. **The host owns the lifecycle handles.** Cordis deliberately has no
//!    runtime-wide shutdown, so [`HostRuntime::shutdown`] disposes plugins in
//!    reverse spawn order, attempt all, and a plugin that fails to load or
//!    unload never stops the host.
//!
//! The worker threads are the host's, not a global: the runtime is `Send`, but
//! it is never polled from the application thread. Only startup and shutdown
//! block, and both are before the first frame and after the last one.

mod bridge;
mod composition;
mod contract;
mod error;
mod ui;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cordis::{Context, FiberHandle, FiberState, PreparedPlugin};
use gpui_kit::{App, Global, Task};

use crate::workspace::contributions;
use crate::workspace::status::{self, PluginPhase};

pub use bridge::{UiInbox, UiMailbox, UiMessage};
pub use composition::{HEARTBEAT, PROVING_PLUGIN, WORKSPACE_PLUGIN, WorkspaceAdapter};
pub use contract::{
    PanelsDeclared, PanelsWithdrawn, WORKSPACE_SERVICE, WorkspaceReady, WorkspaceService,
};
pub use error::CompositionError;

/// How long startup waits for one plugin to settle.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a parked plugin is watched for convergence.
const CONVERGENCE_TIMEOUT: Duration = Duration::from_secs(600);

/// How many worker threads the composition runtime gets.
const RUNTIME_THREADS: usize = 2;

/// One admitted plugin and the handle the host must dispose.
struct PluginRecord {
    name: String,
    handle: FiberHandle,
}

/// One host-owned Cordis runtime.
///
/// Cheap to share: the handle is the runtime's dispatch handle plus the shared
/// lifecycle registry the host must dispose.
pub struct HostRuntime {
    inner: Arc<Inner>,
    worker: tokio::runtime::Runtime,
}

/// What a shared `HostRuntime` actually guards.
struct Inner {
    context: Context,
    mailbox: UiMailbox,
    /// Spawn order, not a map keyed by name: two plugins can be admitted
    /// under the same reported name, and a handle that is overwritten is a
    /// handle that never gets disposed — Cordis is explicit that dropping a
    /// handle is not teardown.
    plugins: Mutex<Vec<PluginRecord>>,
    accepting: AtomicBool,
}

impl HostRuntime {
    /// Start the runtime's worker threads and create its root context.
    ///
    /// The root context is the one every plugin is admitted to, which is what
    /// makes a published service visible to a plugin admitted later.
    pub fn start(mailbox: UiMailbox) -> Result<Self, CompositionError> {
        let worker = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(RUNTIME_THREADS)
            .thread_name("brucebox-composition")
            .enable_time()
            .build()
            .map_err(CompositionError::Executor)?;

        Ok(Self {
            inner: Arc::new(Inner {
                context: Context::new(),
                mailbox,
                plugins: Mutex::new(Vec::new()),
                accepting: AtomicBool::new(true),
            }),
            worker,
        })
    }

    /// The mailbox every plugin reports through.
    pub fn mailbox(&self) -> &UiMailbox {
        &self.inner.mailbox
    }

    /// The root context, for a caller that wants to observe the runtime itself.
    pub fn context(&self) -> &Context {
        &self.inner.context
    }

    /// Admit one plugin and wait for it to settle.
    ///
    /// Returns the phase it settled in: `Active` when apply ran, `Pending` when
    /// a required service is missing. A `Pending` plugin is watched, so the
    /// interface learns when it converges on its own.
    ///
    /// This blocks the calling thread until the plugin settles. Startup uses it
    /// before the first window exists, where a deterministic contribution set is
    /// worth more than a couple of milliseconds.
    pub fn start_plugin(
        &self,
        name: &'static str,
        prepared: PreparedPlugin,
    ) -> Result<PluginPhase, CompositionError> {
        if !self.inner.accepting.load(Ordering::SeqCst) {
            return Err(CompositionError::NotAccepting);
        }

        let (sender, receiver) = tokio::sync::oneshot::channel();
        let inner = self.inner.clone();
        self.worker.spawn(async move {
            let _ = sender.send(inner.admit(name, prepared).await);
        });

        self.worker.block_on(async {
            match tokio::time::timeout(SETTLE_TIMEOUT, receiver).await {
                Ok(Ok(outcome)) => outcome,
                Ok(Err(_)) => Err(CompositionError::Spawn(
                    "the admission task ended without a result".to_owned(),
                )),
                Err(_) => Err(CompositionError::SettleTimeout {
                    plugin: name.to_owned(),
                }),
            }
        })
    }

    /// The plugins this runtime is holding, in spawn order, with the state the
    /// host last saw.
    pub fn plugins(&self) -> Vec<(String, FiberState)> {
        self.lock_plugins()
            .iter()
            .map(|record| (record.name.clone(), record.handle.state()))
            .collect()
    }

    /// Dispose one plugin, which withdraws everything its generation owned.
    pub fn stop_plugin(&self, name: &str) -> Result<(), CompositionError> {
        let record = {
            let mut plugins = self.lock_plugins();
            let index = plugins
                .iter()
                .rposition(|record| record.name == name)
                .ok_or_else(|| {
                    CompositionError::Spawn(format!("plugin `{name}` is not running"))
                })?;
            plugins.remove(index)
        };

        self.worker
            .block_on(record.handle.dispose())
            .map_err(|error| CompositionError::Spawn(error.to_string()))?;
        self.inner.mailbox.phase(name, PluginPhase::Stopped, None);
        Ok(())
    }

    /// Stop accepting work and dispose every plugin, newest first.
    ///
    /// Reverse spawn order is application policy, not a hidden parent/child
    /// relation: a plugin admitted later may depend on an earlier one, so it has
    /// to go first. Attempt-all, because one plugin whose cleanup fails must not
    /// keep the rest from cleaning up. Idempotent.
    pub fn shutdown(&self) {
        if !self.inner.accepting.swap(false, Ordering::SeqCst) {
            return;
        }
        self.inner.mailbox.note("composition runtime stopping");

        let handles: Vec<PluginRecord> = {
            let mut plugins = self.lock_plugins();
            std::mem::take(&mut *plugins)
        };
        let total = handles.len();
        let mailbox = self.inner.mailbox.clone();

        let disposed = self.worker.block_on(async move {
            let mut disposed = 0usize;
            for record in handles.into_iter().rev() {
                let name = record.name;
                match record.handle.dispose().await {
                    Ok(()) => {
                        disposed += 1;
                        mailbox.phase(&name, PluginPhase::Stopped, None);
                    }
                    Err(error) => {
                        mailbox.note(format!("{name} did not dispose cleanly: {error}"));
                    }
                }
            }
            disposed
        });

        self.inner.mailbox.note(format!(
            "composition runtime stopped · {disposed}/{total} plugin(s) disposed"
        ));
    }

    fn lock_plugins(&self) -> std::sync::MutexGuard<'_, Vec<PluginRecord>> {
        self.inner
            .plugins
            .lock()
            .expect("the plugin registry is not poisoned")
    }
}

impl Inner {
    /// Admit one plugin into the root context and record how it settled.
    async fn admit(
        self: Arc<Self>,
        name: &'static str,
        prepared: PreparedPlugin,
    ) -> Result<PluginPhase, CompositionError> {
        self.report(name, PluginPhase::Starting, None);

        let handle = match self.context.spawn(prepared).await {
            Ok(handle) => handle,
            Err(error) => {
                // A plugin that fails to apply is a diagnostic, not a crash: the
                // host keeps running with the other plugins it has.
                let detail = error.to_string();
                self.report(name, PluginPhase::Failed, Some(detail.clone()));
                return Err(CompositionError::Spawn(detail));
            }
        };

        let phase = match handle.state() {
            FiberState::Active => PluginPhase::Active,
            FiberState::Pending => PluginPhase::Pending,
            _ => PluginPhase::Starting,
        };
        let detail = match phase {
            PluginPhase::Pending => {
                let missing = handle.pending_missing();
                Some(if missing.is_empty() {
                    "waiting for its dependencies".to_owned()
                } else {
                    format!("requires {}", missing.join(", "))
                })
            }
            _ => None,
        };

        self.plugins
            .lock()
            .expect("the plugin registry is not poisoned")
            .push(PluginRecord {
                name: name.to_owned(),
                handle: handle.clone(),
            });
        self.report(name, phase, detail);

        // A parked plugin converges on its own once its dependency appears.
        // Waiting for that state publication keeps the interface honest instead
        // of showing `pending` for a plugin that has since applied.
        if phase == PluginPhase::Pending {
            let inner = self.clone();
            tokio::spawn(async move {
                if handle
                    .wait_state(FiberState::Active, CONVERGENCE_TIMEOUT)
                    .await
                    .is_ok()
                {
                    inner.report(name, PluginPhase::Active, None);
                }
            });
        }

        Ok(phase)
    }

    fn report(&self, name: &str, phase: PluginPhase, detail: Option<String>) {
        self.mailbox.phase(name, phase, detail);
    }
}

/// The runtime plus the task that drains its mailbox, as one application global.
///
/// The task is held here for one reason: dropping a GPUI [`Task`] cancels it, and
/// a cancelled drain would leave the interface permanently deaf.
pub struct RuntimeHost {
    runtime: HostRuntime,
    _drain: Task<()>,
    _app_quit: gpui_kit::Subscription,
}

impl Global for RuntimeHost {}

/// The host runtime, if [`install`] has run.
pub fn runtime(cx: &App) -> Option<&HostRuntime> {
    cx.try_global::<RuntimeHost>().map(|host| &host.runtime)
}

/// Start the composition runtime and submit the host's own plugins.
///
/// Runs before the first window is opened: the initial layout is resolved
/// against the catalog, and the catalog has to have the contributions in it
/// before that happens. Anything a plugin publishes after this point still
/// arrives, through the drain task.
pub fn install(cx: &mut App) -> Result<(), CompositionError> {
    status::init(cx);

    let (mailbox, mut inbox) = UiInbox::channel();
    let runtime = HostRuntime::start(mailbox.clone())?;

    // Workspace service first, then the contributions that require it: the
    // dependency is satisfied inside `start_plugin`, so a plugin that declares
    // panels is Active rather than Pending when its contributions are declared.
    runtime.start_plugin(
        WORKSPACE_PLUGIN,
        WorkspaceAdapter::prepared(mailbox.clone()),
    )?;
    runtime.start_plugin(
        PROVING_PLUGIN,
        composition::ProvingContributions::prepared(contributions::proving_declaration()),
    )?;

    // Apply startup's messages here, synchronously, so the first frame already
    // knows about every panel the runtime just contributed.
    for message in inbox.drain() {
        ui::apply(cx, message);
    }

    let drain = ui::install_drain(cx, inbox);
    let app_quit = cx.on_app_quit(|cx| {
        if let Some(host) = cx.try_global::<RuntimeHost>() {
            host.runtime.shutdown();
        }
        async {}
    });

    cx.set_global(RuntimeHost {
        runtime,
        _drain: drain,
        _app_quit: app_quit,
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::composition::{ProvingContributions, WorkspaceAdapter};
    use super::*;
    use crate::workspace::contributions;
    use cordis::Plugin;
    use std::convert::Infallible;
    use std::time::Instant;

    /// Collect messages until `done` is satisfied or the budget runs out.
    ///
    /// The runtime reports from its own threads, so a test waits on the mailbox
    /// — the same path the application thread uses.
    fn wait_for(
        inbox: &mut UiInbox,
        budget: Duration,
        mut done: impl FnMut(&[UiMessage]) -> bool,
    ) -> Vec<UiMessage> {
        let start = Instant::now();
        let mut messages = Vec::new();
        loop {
            messages.extend(inbox.drain());
            if done(&messages) || start.elapsed() > budget {
                return messages;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn phase_of(messages: &[UiMessage], plugin: &str) -> Option<PluginPhase> {
        messages.iter().rev().find_map(|message| match message {
            UiMessage::Phase {
                plugin: reported,
                phase,
                ..
            } if reported == plugin => Some(*phase),
            _ => None,
        })
    }

    #[test]
    fn the_runtime_admits_the_host_plugins_and_their_contributions_reach_the_interface() {
        let (mailbox, mut inbox) = UiInbox::channel();
        let runtime = HostRuntime::start(mailbox.clone()).expect("the runtime starts");

        assert_eq!(
            runtime
                .start_plugin(
                    WORKSPACE_PLUGIN,
                    WorkspaceAdapter::prepared(mailbox.clone())
                )
                .expect("the adapter admits"),
            PluginPhase::Active
        );
        assert_eq!(
            runtime
                .start_plugin(
                    PROVING_PLUGIN,
                    ProvingContributions::prepared(contributions::proving_declaration()),
                )
                .expect("the contributions admit"),
            PluginPhase::Active
        );

        let messages = wait_for(&mut inbox, Duration::from_secs(5), |messages| {
            messages
                .iter()
                .any(|message| matches!(message, UiMessage::Declared(_)))
        });

        let declared = messages
            .iter()
            .find_map(|message| match message {
                UiMessage::Declared(declaration) => Some(declaration),
                _ => None,
            })
            .expect("the plugin declared its contributions");
        assert_eq!(declared.plugin().as_str(), PROVING_PLUGIN);
        assert_eq!(
            declared.panels().len(),
            contributions::proving_declaration().panels().len()
        );

        // Service consumption and async work, both generation-owned.
        assert!(
            messages.iter().any(|message| matches!(
                message,
                UiMessage::Note(note) if note.contains("brucebox.workspace")
            )),
            "the adapter reports the service it published"
        );
        assert!(
            messages.iter().any(|message| matches!(
                message,
                UiMessage::Note(note) if note.contains("tick 0")
            )),
            "the plugin's task starts reporting as soon as it applies"
        );
        assert_eq!(
            phase_of(&messages, WORKSPACE_PLUGIN),
            Some(PluginPhase::Active)
        );
        assert_eq!(
            phase_of(&messages, PROVING_PLUGIN),
            Some(PluginPhase::Active)
        );

        runtime.shutdown();
    }

    #[test]
    fn a_plugin_that_requires_the_workspace_service_waits_for_it_then_converges() {
        let (mailbox, mut inbox) = UiInbox::channel();
        let runtime = HostRuntime::start(mailbox.clone()).expect("the runtime starts");

        // Submitted before anything publishes the service: the requirement is a
        // lifecycle prerequisite, so apply must not run.
        assert_eq!(
            runtime
                .start_plugin(
                    PROVING_PLUGIN,
                    ProvingContributions::prepared(contributions::proving_declaration()),
                )
                .expect("the plugin is admitted as pending"),
            PluginPhase::Pending
        );

        let parked = wait_for(&mut inbox, Duration::from_secs(2), |messages| {
            phase_of(messages, PROVING_PLUGIN) == Some(PluginPhase::Pending)
        });
        assert_eq!(
            phase_of(&parked, PROVING_PLUGIN),
            Some(PluginPhase::Pending)
        );
        assert!(
            parked.iter().any(|message| matches!(
                message,
                UiMessage::Phase { detail: Some(detail), .. }
                    if detail.contains(WORKSPACE_SERVICE)
            )),
            "a pending plugin says which service it is waiting for"
        );
        assert!(
            !parked
                .iter()
                .any(|message| matches!(message, UiMessage::Declared(_))),
            "a pending plugin never ran apply"
        );

        // Publishing the service converges the parked plugin on its own.
        assert_eq!(
            runtime
                .start_plugin(
                    WORKSPACE_PLUGIN,
                    WorkspaceAdapter::prepared(mailbox.clone())
                )
                .expect("the adapter admits"),
            PluginPhase::Active
        );

        let converged = wait_for(&mut inbox, Duration::from_secs(5), |messages| {
            phase_of(messages, PROVING_PLUGIN) == Some(PluginPhase::Active)
                && messages
                    .iter()
                    .any(|message| matches!(message, UiMessage::Declared(_)))
        });
        assert_eq!(
            phase_of(&converged, PROVING_PLUGIN),
            Some(PluginPhase::Active),
            "the parked plugin applied once its dependency appeared"
        );
        assert!(
            converged
                .iter()
                .any(|message| matches!(message, UiMessage::Declared(_))),
            "convergence is what delivered the contribution"
        );

        runtime.shutdown();
    }

    /// A plugin that records what it applied and cleaned up, in order.
    struct Recorder {
        name: &'static str,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Plugin for Recorder {
        type Config = ();
        type Input = Arc<Mutex<Vec<String>>>;
        type PrepareError = Infallible;
        type ApplyError = CompositionError;

        fn name(&self) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Borrowed(self.name)
        }

        fn prepare(&self, (): ()) -> Result<Self::Input, Self::PrepareError> {
            Ok(self.log.clone())
        }

        async fn apply(&self, ctx: Context, log: &Self::Input) -> Result<(), Self::ApplyError> {
            let name = self.name;
            log.lock()
                .expect("the log is not poisoned")
                .push(format!("apply {name}"));
            let log = log.clone();
            ctx.effect_sync(move || {
                log.lock()
                    .expect("the log is not poisoned")
                    .push(format!("cleanup {name}"));
            })
            .map_err(|error| CompositionError::Effect(error.to_string()))?;
            Ok(())
        }
    }

    /// A plugin whose apply always fails.
    struct Broken;

    impl Plugin for Broken {
        type Config = ();
        type Input = ();
        type PrepareError = Infallible;
        type ApplyError = CompositionError;

        fn name(&self) -> std::borrow::Cow<'_, str> {
            std::borrow::Cow::Borrowed("brucebox.test.broken")
        }

        fn prepare(&self, (): ()) -> Result<Self::Input, Self::PrepareError> {
            Ok(())
        }

        async fn apply(&self, _: Context, (): &()) -> Result<(), Self::ApplyError> {
            Err(CompositionError::Event(
                "this plugin always fails".to_owned(),
            ))
        }
    }

    #[test]
    fn shutdown_disposes_plugins_newest_first_and_runs_their_cleanup() {
        let (mailbox, _inbox) = UiInbox::channel();
        let runtime = HostRuntime::start(mailbox.clone()).expect("the runtime starts");
        let log = Arc::new(Mutex::new(Vec::new()));

        for name in ["brucebox.test.first", "brucebox.test.second"] {
            runtime
                .start_plugin(
                    "brucebox.test.recorder",
                    PreparedPlugin::from_input(
                        Recorder {
                            name,
                            log: log.clone(),
                        },
                        log.clone(),
                    ),
                )
                .expect("the recorder admits");
        }

        assert_eq!(
            *log.lock().unwrap(),
            vec!["apply brucebox.test.first", "apply brucebox.test.second"]
        );

        runtime.shutdown();

        assert_eq!(
            *log.lock().unwrap(),
            vec![
                "apply brucebox.test.first",
                "apply brucebox.test.second",
                "cleanup brucebox.test.second",
                "cleanup brucebox.test.first",
            ],
            "reverse spawn order, and every generation's cleanup ran"
        );
        assert!(runtime.plugins().is_empty(), "no handle outlives shutdown");

        // Idempotent, and closed to new work.
        runtime.shutdown();
        assert!(matches!(
            runtime.start_plugin(
                WORKSPACE_PLUGIN,
                WorkspaceAdapter::prepared(mailbox.clone())
            ),
            Err(CompositionError::NotAccepting)
        ));
    }

    #[test]
    fn a_plugin_that_fails_to_apply_is_reported_and_does_not_stop_the_runtime() {
        let (mailbox, mut inbox) = UiInbox::channel();
        let runtime = HostRuntime::start(mailbox.clone()).expect("the runtime starts");

        let error = runtime
            .start_plugin(
                "brucebox.test.broken",
                PreparedPlugin::from_input(Broken, ()),
            )
            .expect_err("a failing apply is reported");
        assert!(matches!(error, CompositionError::Spawn(_)));

        let reported = wait_for(&mut inbox, Duration::from_secs(2), |messages| {
            phase_of(messages, "brucebox.test.broken") == Some(PluginPhase::Failed)
        });
        assert_eq!(
            phase_of(&reported, "brucebox.test.broken"),
            Some(PluginPhase::Failed)
        );

        // The host is unaffected: the next plugin applies normally.
        assert_eq!(
            runtime
                .start_plugin(
                    WORKSPACE_PLUGIN,
                    WorkspaceAdapter::prepared(mailbox.clone())
                )
                .expect("the adapter still admits"),
            PluginPhase::Active
        );
        assert!(
            runtime
                .plugins()
                .iter()
                .any(|(name, state)| name == WORKSPACE_PLUGIN && *state == FiberState::Active),
            "the adapter is still active after the failure"
        );

        runtime.shutdown();
    }
}

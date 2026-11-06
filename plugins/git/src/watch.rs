//! The plugin's composition contribution: keep the repository fresh.
//!
//! The watcher is what makes the plugins cooperate without either of them knowing
//! the other exists: it re-reads the repository on a timer and publishes the
//! snapshot on the contract bus, so a consumer sees changes whether or not any Git
//! panel happens to be open.
//!
//! # Why a thread and not a task
//!
//! A plugin's future is polled by the *host's* executor, which belongs to the
//! host's copy of the runtime. Awaiting a timer from the plugin's own dependency
//! graph would ask that other build's reactor for a timer it is not driving. A
//! plain thread with `std::thread::sleep` has no such question, costs one thread
//! for the life of the plugin, and stops on a flag the generation owns.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use brucebox_sdk::cordis::{Context, InjectSpec, Plugin, PreparedPlugin};

use crate::store::GitStore;

/// How often the repository is re-read.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Why the watcher could not start.
#[derive(Debug)]
pub struct WatcherError(String);

impl std::fmt::Display for WatcherError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "the Git watcher could not start: {}", self.0)
    }
}

impl std::error::Error for WatcherError {}

struct GitWatcher;

impl Plugin for GitWatcher {
    type Config = ();
    type Input = ();
    type PrepareError = std::convert::Infallible;
    type ApplyError = WatcherError;

    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("brucebox.git")
    }

    fn inject(&self) -> InjectSpec {
        // Nothing required: the Git plugin is a publisher, not a consumer, and it
        // keeps working when every other plugin is missing.
        InjectSpec::none()
    }

    fn prepare(&self, (): ()) -> Result<Self::Input, Self::PrepareError> {
        Ok(())
    }

    async fn apply(&self, ctx: Context, (): &Self::Input) -> Result<(), Self::ApplyError> {
        let store = GitStore::shared();

        // Open the repository the host was pointed at, unless a panel already did.
        if store.state().root.is_none()
            && let Some(root) = GitStore::default_root()
        {
            let _ = store.open(&root);
        }

        let stop = Arc::new(AtomicBool::new(false));
        let running = stop.clone();
        std::thread::Builder::new()
            .name("brucebox-git-poll".to_owned())
            .spawn(move || {
                while !running.load(Ordering::Relaxed) {
                    // Sleep in small steps so a plugin that is going away stops
                    // promptly instead of after a whole interval.
                    for _ in 0..POLL_INTERVAL.as_millis() / 100 {
                        if running.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    store.refresh();
                }
            })
            .map_err(|error| WatcherError(error.to_string()))?;

        // The generation owns the thread: unloading the plugin stops it.
        stop_effect(&ctx, stop)?;
        Ok(())
    }
}

/// Register the stop flag with the plugin's generation.
///
/// Closing the generation stops the poller and clears the store, so a stopped Git
/// plugin leaves no stale repository behind for the panels to show.
fn stop_effect(ctx: &Context, stop: Arc<AtomicBool>) -> Result<(), WatcherError> {
    let store = GitStore::shared();
    ctx.effect_sync(move || {
        stop.store(true, Ordering::Relaxed);
        store.clear();
    })
    .map(|_| ())
    .map_err(|error| WatcherError(error.to_string()))
}

/// Seal the plugin's contribution for the host to admit.
pub fn prepare() -> PreparedPlugin {
    PreparedPlugin::from_input(GitWatcher, ())
}

//! The plugin's contribution to the host's composition runtime.
//!
//! Agent Chat publishes one typed service — the conversation transcript other
//! plugins can read and append to — and a typed event for appends. It is
//! deliberately small: the runtime integration is the point here, and stage 7's
//! Git plugin is what consumes this service.
//!
//! The plugin never creates a runtime. It returns a sealed [`PreparedPlugin`];
//! the host admits it into the one host-owned runtime, owns the fiber, and gives
//! the plugin its `Context`. Everything registered in `apply` is owned by that
//! generation and cleaned up when it closes.

use std::fmt;
use std::sync::{Arc, Mutex};

use brucebox_sdk::cordis::event::observer_sync;
use brucebox_sdk::cordis::{
    Context, Event, InjectSpec, Plugin, PreparedPlugin, Routing, Service,
};

/// The service name this plugin publishes.
pub const SESSION_SERVICE: &str = "brucebox.agent-chat.session";

/// The transcript other plugins can consume.
///
/// A real conversation store replaces the in-memory lines later; the contract —
/// a name, a type, and a place in the runtime — stays the same.
#[derive(Debug, Default)]
pub struct Session {
    messages: Mutex<Vec<String>>,
}

impl Session {
    /// Append one line and return the transcript length.
    pub fn push(&self, text: impl Into<String>) -> usize {
        let mut messages = self.messages.lock().expect("the transcript is not poisoned");
        messages.push(text.into());
        messages.len()
    }

    /// Everything said so far.
    ///
    /// Consumed by the plugin that reads this session in stage 7; the service is
    /// published now so that contract has somewhere to land.
    #[allow(dead_code)]
    pub fn messages(&self) -> Vec<String> {
        self.messages
            .lock()
            .expect("the transcript is not poisoned")
            .clone()
    }
}

impl Service for Session {
    const NAME: &'static str = SESSION_SERVICE;
}

/// A line was appended to the transcript.
pub struct MessageAppended;

impl Event for MessageAppended {
    const NAME: &'static str = "brucebox.agent-chat.message-appended";
    type Args = String;
    type Output = ();
}

/// Why the plugin's contribution could not be applied.
#[derive(Debug)]
pub struct SessionError(String);

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "agent chat could not join the runtime: {}", self.0)
    }
}

impl std::error::Error for SessionError {}

/// The plugin's Cordis behavior.
struct AgentChatRuntime;

impl Plugin for AgentChatRuntime {
    type Config = ();
    type Input = ();
    type PrepareError = std::convert::Infallible;
    type ApplyError = SessionError;

    fn name(&self) -> std::borrow::Cow<'_, str> {
        std::borrow::Cow::Borrowed("brucebox.agent-chat")
    }

    fn inject(&self) -> InjectSpec {
        // Nothing is required: the transcript is this plugin's own.
        InjectSpec::none()
    }

    fn prepare(&self, (): ()) -> Result<Self::Input, Self::PrepareError> {
        Ok(())
    }

    async fn apply(&self, ctx: Context, (): &Self::Input) -> Result<(), Self::ApplyError> {
        let session = Arc::new(Session::default());
        // Published into this generation: it disappears when the plugin unloads,
        // and dependants converge instead of reading a dead handle.
        let _publication = ctx
            .provide::<Session>(session.clone())
            .map_err(|error| SessionError(error.to_string()))?;

        let transcript = session.clone();
        let _listener = ctx
            .on::<MessageAppended, _>(observer_sync(move |_, text| {
                transcript.push(text);
                Ok::<(), std::convert::Infallible>(())
            }))
            .map_err(|error| SessionError(error.to_string()))?;

        // The generation owns the transcript: closing it clears the session.
        let owned = session.clone();
        ctx.effect_sync(move || {
            owned
                .messages
                .lock()
                .expect("the transcript is not poisoned")
                .clear();
        })
        .map_err(|error| SessionError(error.to_string()))?;

        ctx.emit::<MessageAppended>(Routing::Unscoped, "session opened".to_owned())
            .await
            .map_err(|error| SessionError(error.to_string()))?;

        Ok(())
    }
}

/// Seal the plugin's contribution for the host to admit.
pub fn prepare() -> PreparedPlugin {
    PreparedPlugin::from_input(AgentChatRuntime, ())
}

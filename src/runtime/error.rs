//! Why the composition runtime could not do what the host asked.

use std::fmt;

/// A failure at the host's side of the composition runtime.
///
/// Plugin failures themselves are not modelled here: Cordis normalizes those
/// into an opaque `PluginFailure` and the host reports it as a status line, so a
/// plugin that misbehaves never takes the runtime down with it.
#[derive(Debug)]
pub enum CompositionError {
    /// The worker threads could not be started.
    Executor(std::io::Error),
    /// A plugin could not be admitted.
    Spawn(String),
    /// A plugin did not settle inside the start timeout.
    SettleTimeout {
        /// The plugin that did not settle.
        plugin: String,
    },
    /// A service could not be published.
    Service(String),
    /// An event could not be dispatched.
    Event(String),
    /// A listener could not be registered.
    Listener(String),
    /// A generation-owned task or cleanup could not be registered.
    Effect(String),
    /// The runtime has begun shutting down and accepts no new work.
    NotAccepting,
}

impl fmt::Display for CompositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Executor(error) => {
                write!(formatter, "cannot start the runtime executor: {error}")
            }
            Self::Spawn(detail) => write!(formatter, "plugin admission failed: {detail}"),
            Self::SettleTimeout { plugin } => {
                write!(formatter, "plugin `{plugin}` did not settle in time")
            }
            Self::Service(detail) => write!(formatter, "service operation failed: {detail}"),
            Self::Event(detail) => write!(formatter, "event dispatch failed: {detail}"),
            Self::Listener(detail) => write!(formatter, "listener registration failed: {detail}"),
            Self::Effect(detail) => write!(formatter, "effect registration failed: {detail}"),
            Self::NotAccepting => formatter.write_str("the runtime is shutting down"),
        }
    }
}

impl std::error::Error for CompositionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Executor(error) => Some(error),
            _ => None,
        }
    }
}

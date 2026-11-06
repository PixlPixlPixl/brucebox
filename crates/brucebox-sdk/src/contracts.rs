//! Typed contracts over the host's cross-plugin bus.
//!
//! A contract is a Rust type in a crate both plugins depend on, plus a topic name.
//! The type gives both sides the same values; the topic name is what actually
//! crosses, because two separately compiled plugins do not share `TypeId`s and so
//! cannot hand each other a `dyn` value (ADR 0007). Values travel as JSON on the
//! bus: the contract crate owns the type, the SDK owns the encoding.
//!
//! ```ignore
//! // in a shared contract crate
//! pub const TOPIC: &str = "example.counter@1";
//! #[derive(serde::Serialize, serde::Deserialize)]
//! pub struct Counter { pub value: u32 }
//!
//! // publisher
//! wayfinder::contracts::publish(TOPIC, &Counter { value: 3 })?;
//!
//! // consumer
//! let current = contracts::read::<Counter>(TOPIC);
//! let subscription = contracts::subscribe(TOPIC, |counter: Counter| { /* … */ });
//! ```
//!
//! # What the bus guarantees
//!
//! - one value per topic, the most recent publication, and the last write wins;
//! - a publication with byte-identical content is not announced again, so a
//!   polling publisher is quiet when nothing changed;
//! - a publisher is not called back for its own publication;
//! - a handler runs on the publishing thread and must be quick. It receives an
//!   owned value, so anything it needs to keep, it keeps.
//!
//! # What it does not
//!
//! The bus is not a request/response channel and not a queue: there is no
//! delivery for a consumer that subscribed late, which is why [`read`] exists. A
//! topic with no value reads as `None`, which is also how a consumer notices that
//! the publishing plugin is not installed.

use std::ffi::c_void;

use serde::Serialize;
use serde::de::DeserializeOwned;

use brucebox_plugin_api::{Bytes, ContractStatus, HostContext};

/// Why a contract value could not be published or decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractError {
    /// The plugin was loaded without a host context, so there is no bus.
    NoHost,
    /// The host refused the call for a reason the plugin can act on.
    Refused(ContractStatus),
    /// The value could not be encoded.
    Encode(String),
}

impl std::fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoHost => formatter.write_str("the host did not provide a contract bus"),
            Self::Refused(status) => {
                write!(formatter, "the contract bus refused the call: {status:?}")
            }
            Self::Encode(detail) => write!(formatter, "cannot encode the contract value: {detail}"),
        }
    }
}

impl std::error::Error for ContractError {}

/// The host context this plugin was loaded with, if any.
pub fn host() -> Option<HostContext> {
    crate::HOST.get().copied()
}

/// Publish `value` as the current value of `topic`.
pub fn publish<T: Serialize>(topic: &str, value: &T) -> Result<(), ContractError> {
    let host = host().ok_or(ContractError::NoHost)?;
    let payload =
        serde_json::to_vec(value).map_err(|error| ContractError::Encode(error.to_string()))?;
    let status = (host.bus.publish)(
        host.bus.user,
        Bytes::of(topic.as_bytes()),
        Bytes::of(&payload),
    );
    match status {
        ContractStatus::Ok => Ok(()),
        other => Err(ContractError::Refused(other)),
    }
}

/// The current value of `topic`, if any plugin published one this session.
pub fn read<T: DeserializeOwned>(topic: &str) -> Option<T> {
    let host = host()?;
    let topic_bytes = Bytes::of(topic.as_bytes());
    let length = (host.bus.read)(host.bus.user, topic_bytes, std::ptr::null_mut(), 0);
    if length < 0 {
        return None;
    }
    let mut buffer = vec![0u8; length as usize];
    let copied = (host.bus.read)(
        host.bus.user,
        topic_bytes,
        buffer.as_mut_ptr(),
        buffer.len(),
    );
    if copied != length {
        return None;
    }
    serde_json::from_slice(&buffer).ok()
}

/// A live subscription. Dropping it stops delivery.
pub struct Subscription {
    id: u64,
    sink: *mut Handler,
    /// Kept for the `Drop` call and for diagnostics.
    topic: String,
}

type Handler = Box<dyn FnMut(&[u8]) + Send>;

/// Subscribe to `topic`, calling `handler` whenever its value changes.
///
/// The handler runs on the thread that published. It gets an owned, decoded value,
/// so a slow handler delays the publisher, not the bus.
pub fn subscribe<T, F>(topic: &str, mut handler: F) -> Result<Subscription, ContractError>
where
    T: DeserializeOwned + Send + 'static,
    F: FnMut(T) + Send + 'static,
{
    let host = host().ok_or(ContractError::NoHost)?;

    let boxed: Handler = Box::new(move |payload: &[u8]| {
        if let Ok(value) = serde_json::from_slice::<T>(payload) {
            handler(value);
        }
    });
    let sink = Box::into_raw(Box::new(boxed));

    let id = (host.bus.subscribe)(
        host.bus.user,
        Bytes::of(topic.as_bytes()),
        deliver::<T>,
        sink as *mut c_void,
    );
    if id == 0 {
        // SAFETY: the pointer came from `Box::into_raw` just above and was never
        // handed to the host.
        unsafe { drop(Box::from_raw(sink)) };
        return Err(ContractError::Refused(ContractStatus::Refused));
    }

    Ok(Subscription {
        id,
        sink,
        topic: topic.to_owned(),
    })
}

/// The trampoline the bus calls: hands the bytes to the boxed handler.
extern "C" fn deliver<T>(sink: *mut c_void, _topic: Bytes, payload: Bytes) {
    // SAFETY: the sink was created by `subscribe` from a `Box<Handler>` and is
    // freed only after the host has forgotten the subscription.
    let handler = unsafe { &mut *(sink as *mut Handler) };
    // SAFETY: the payload belongs to the publisher and is valid for this call.
    handler(unsafe { payload.as_slice() });
    let _ = std::marker::PhantomData::<T>;
}

// SAFETY: a subscription is an id plus a boxed handler that is already `Send`.
// The raw `sink` pointer is only ever dereferenced by the bus trampoline, which
// the host calls on the publishing thread; `unsubscribe` happens exactly once, in
// `Drop`, after which the box is freed. Moving a `Subscription` to another thread
// therefore transfers no aliasing: at any moment the bus holds exactly one owner
// of `sink`.
unsafe impl Send for Subscription {}

impl Subscription {
    /// The topic this subscription listens to.
    pub fn topic(&self) -> &str {
        &self.topic
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(host) = host() {
            (host.bus.unsubscribe)(host.bus.user, self.id);
        }
        // SAFETY: the pointer came from `Box::into_raw` in `subscribe`, and the
        // host no longer holds it after the unsubscribe above.
        unsafe { drop(Box::from_raw(self.sink)) };
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Subscription")
            .field("topic", &self.topic)
            .field("id", &self.id)
            .finish()
    }
}

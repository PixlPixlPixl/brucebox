//! The host's cross-plugin contract bus.
//!
//! Plugins cooperate by publishing data on named topics and reading or
//! subscribing to them. The *types* live in a shared contract crate, so both
//! sides see the same Rust values; the bytes are what actually cross, because a
//! service value published by one plugin cannot be downcast by another (ADR 0007
//! has the measurement).
//!
//! # Rules the bus keeps
//!
//! - A topic holds **one** value: the most recent publication. Last writer wins,
//!   and the store records *which* plugin published it so the interface can say
//!   where a value came from.
//! - Publishing bytes identical to the current value updates nothing and notifies
//!   nobody. A plugin that polls can therefore publish on every tick and still be
//!   a well-behaved publisher.
//! - Subscribers are notified on the publishing thread, after the lock is
//!   released, and a publisher is never notified of its own publication.
//! - A callback receives borrowed bytes that are only valid during the call.
//! - The bus is process-wide and reachable from any thread: a plugin's Cordis task
//!   publishes from a runtime thread while its panels render on the application
//!   thread.
//!
//! The bus is not a message queue: nothing is delivered late, and reading a topic
//! is how a *new* consumer learns the current value.

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use brucebox_plugin_api::{Bytes, CONTRACT_PAYLOAD_LIMIT, ContractNotify, ContractStatus};

/// The host's contract store.
#[derive(Default)]
pub struct ContractRegistry {
    inner: Mutex<Inner>,
    next_subscription: AtomicU64,
}

#[derive(Debug, Default)]
struct Inner {
    topics: BTreeMap<String, Topic>,
}

#[derive(Debug)]
struct Topic {
    payload: Vec<u8>,
    publisher: String,
    subscribers: Vec<Subscriber>,
}

/// One live subscription.
#[derive(Debug, Clone, Copy)]
struct Subscriber {
    id: u64,
    notify: ContractNotify,
    sink: *mut c_void,
}

// SAFETY: a subscriber is only ever invoked through `ContractNotify` with the
// `sink` the subscriber supplied, on the thread that published. The pointer is
// owned by the subscriber (the SDK keeps it alive for the subscription's
// lifetime), so moving the record between threads transfers no ownership.
unsafe impl Send for Subscriber {}

impl std::fmt::Debug for ContractRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        formatter
            .debug_struct("ContractRegistry")
            .field("topics", &inner.topics.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ContractRegistry {
    /// Publish `payload` as `publisher` on `topic`.
    pub fn publish(&self, publisher: &str, topic: &str, payload: &[u8]) -> ContractStatus {
        if let Err(status) = check_topic(topic).and_then(|()| check_payload(payload)) {
            return status;
        }

        let (changed, subscribers) = {
            let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
            let entry = inner
                .topics
                .entry(topic.to_owned())
                .or_insert_with(|| Topic {
                    payload: Vec::new(),
                    publisher: String::new(),
                    subscribers: Vec::new(),
                });
            let changed = entry.payload != payload;
            if changed {
                entry.payload.clear();
                entry.payload.extend_from_slice(payload);
            }
            entry.publisher = publisher.to_owned();

            // A publisher is not notified of its own publication: it just wrote
            // the value, and a reentrant callback would make the bus surprising.
            let subscribers: Vec<Subscriber> = if changed {
                entry.subscribers.clone()
            } else {
                Vec::new()
            };
            (changed, subscribers)
        };
        let _ = changed;

        let topic_bytes = Bytes::of(topic.as_bytes());
        let payload_bytes = Bytes::of(payload);
        for subscriber in subscribers {
            (subscriber.notify)(subscriber.sink, topic_bytes, payload_bytes);
        }
        ContractStatus::Ok
    }

    /// Copy the current value of `topic` into `out`, returning its length.
    ///
    /// With a null `out` this only reports the length. `-1` means the topic has
    /// no value at all.
    ///
    /// # Safety
    ///
    /// `out` must be null or point at `capacity` writable bytes.
    pub unsafe fn read(&self, topic: &str, out: *mut u8, capacity: usize) -> isize {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let Some(entry) = inner.topics.get(topic) else {
            return -1;
        };
        let length = entry.payload.len();
        if out.is_null() || capacity < length {
            return length as isize;
        }
        // SAFETY: the caller supplied a buffer of at least `length` bytes and a
        // non-null pointer; `length` comes from the value being copied.
        unsafe { std::ptr::copy_nonoverlapping(entry.payload.as_ptr(), out, length) };
        length as isize
    }

    /// Which plugin published the current value of `topic`, if any.
    pub fn publisher(&self, topic: &str) -> Option<String> {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner.topics.get(topic).map(|entry| entry.publisher.clone())
    }

    /// The topics that currently hold a value.
    pub fn topics(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner.topics.keys().cloned().collect()
    }

    /// Subscribe to `topic`, returning an id that can be cancelled.
    pub fn subscribe(
        &self,
        topic: &str,
        notify: ContractNotify,
        sink: *mut c_void,
    ) -> Result<u64, ContractStatus> {
        check_topic(topic)?;
        let id = self.next_subscription.fetch_add(1, Ordering::Relaxed) + 1;
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let entry = inner
            .topics
            .entry(topic.to_owned())
            .or_insert_with(|| Topic {
                payload: Vec::new(),
                publisher: String::new(),
                subscribers: Vec::new(),
            });
        entry.subscribers.push(Subscriber { id, notify, sink });
        Ok(id)
    }

    /// Cancel a subscription. Unknown ids are ignored.
    pub fn unsubscribe(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        for entry in inner.topics.values_mut() {
            entry.subscribers.retain(|subscriber| subscriber.id != id);
        }
    }
}

fn check_topic(topic: &str) -> Result<(), ContractStatus> {
    if topic.is_empty() || topic.len() > 128 {
        return Err(ContractStatus::BadTopic);
    }
    if !topic
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || ".-_/@".contains(character))
    {
        return Err(ContractStatus::BadTopic);
    }
    Ok(())
}

fn check_payload(payload: &[u8]) -> Result<(), ContractStatus> {
    if payload.len() > CONTRACT_PAYLOAD_LIMIT {
        return Err(ContractStatus::TooLarge);
    }
    Ok(())
}

/// What the host installs as one plugin's `user` pointer.
///
/// It pairs the registry with the plugin's id, which is how a publication is
/// attributed without the plugin ever naming itself.
#[derive(Debug)]
pub struct PluginBusLink {
    registry: Arc<ContractRegistry>,
    plugin_id: String,
}

impl PluginBusLink {
    /// Link `plugin_id` to the process-wide registry.
    pub fn new(plugin_id: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            registry: registry(),
            plugin_id: plugin_id.into(),
        })
    }

    /// The plugin this link belongs to.
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }
}

/// The process-wide registry, created on first use.
pub fn registry() -> Arc<ContractRegistry> {
    static REGISTRY: OnceLock<Arc<ContractRegistry>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| Arc::new(ContractRegistry::default()))
        .clone()
}

impl PluginBusLink {
    /// The ABI context this plugin is loaded with.
    pub fn host_context(self: &Arc<Self>) -> brucebox_plugin_api::HostContext {
        brucebox_plugin_api::HostContext {
            api_version: brucebox_plugin_api::API_VERSION,
            bus: brucebox_plugin_api::ContractBus {
                user: Arc::as_ptr(self) as *mut c_void,
                publish: publish_shim,
                read: read_shim,
                subscribe: subscribe_shim,
                unsubscribe: unsubscribe_shim,
            },
        }
    }
}

/// # Safety
///
/// `user` must be the pointer [`PluginBusLink::host_context`] installed.
unsafe fn link<'a>(user: *mut c_void) -> &'a PluginBusLink {
    // SAFETY: the host keeps the `Arc<PluginBusLink>` alive for the life of the
    // process (it is owned by the plugin's record), so the pointer is valid for
    // every call that carries it.
    unsafe { &*(user as *const PluginBusLink) }
}

extern "C" fn publish_shim(user: *mut c_void, topic: Bytes, payload: Bytes) -> ContractStatus {
    // SAFETY: the host owns `user`; see `link`.
    let link = unsafe { link(user) };
    // SAFETY: both ranges belong to the caller and live for this call.
    let (topic, payload) = unsafe { (topic.as_slice(), payload.as_slice()) };
    let Ok(topic) = std::str::from_utf8(topic) else {
        return ContractStatus::BadTopic;
    };
    link.registry.publish(&link.plugin_id, topic, payload)
}

extern "C" fn read_shim(user: *mut c_void, topic: Bytes, out: *mut u8, capacity: usize) -> isize {
    // SAFETY: the host owns `user`; see `link`.
    let link = unsafe { link(user) };
    // SAFETY: the range belongs to the caller and lives for this call.
    let Ok(topic) = std::str::from_utf8(unsafe { topic.as_slice() }) else {
        return -1;
    };
    // SAFETY: the ABI documents that `out` is either null or `capacity` writable
    // bytes, and the plugin SDK is the only caller.
    unsafe { link.registry.read(topic, out, capacity) }
}

extern "C" fn subscribe_shim(
    user: *mut c_void,
    topic: Bytes,
    notify: ContractNotify,
    sink: *mut c_void,
) -> u64 {
    // SAFETY: the host owns `user`; see `link`.
    let link = unsafe { link(user) };
    // SAFETY: the range belongs to the caller and lives for this call.
    let Ok(topic) = std::str::from_utf8(unsafe { topic.as_slice() }) else {
        return 0;
    };
    link.registry
        .subscribe(topic, notify, sink)
        .unwrap_or_default()
}

extern "C" fn unsubscribe_shim(user: *mut c_void, id: u64) {
    // SAFETY: the host owns `user`; see `link`.
    let link = unsafe { link(user) };
    link.registry.unsubscribe(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// A sink that records every notification it receives.
    struct Recorder {
        seen: StdMutex<Vec<(String, Vec<u8>)>>,
    }

    extern "C" fn record(user: *mut c_void, topic: Bytes, payload: Bytes) {
        // SAFETY: the test installs a `&Recorder` as the sink and keeps it alive.
        let recorder = unsafe { &*(user as *const Recorder) };
        let topic = String::from_utf8_lossy(unsafe { topic.as_slice() }).into_owned();
        let payload = unsafe { payload.as_slice() }.to_vec();
        recorder.seen.lock().unwrap().push((topic, payload));
    }

    fn registry() -> ContractRegistry {
        ContractRegistry::default()
    }

    #[test]
    fn a_publication_is_readable_and_attributed_to_its_publisher() {
        let bus = registry();

        assert_eq!(
            bus.publish("brucebox.git", "brucebox.git.repository@0.1", b"one"),
            ContractStatus::Ok
        );
        assert_eq!(
            bus.publisher("brucebox.git.repository@0.1").as_deref(),
            Some("brucebox.git")
        );

        let mut buffer = [0u8; 8];
        let length = unsafe {
            bus.read(
                "brucebox.git.repository@0.1",
                buffer.as_mut_ptr(),
                buffer.len(),
            )
        };
        assert_eq!(length, 3);
        assert_eq!(&buffer[..3], b"one");
    }

    #[test]
    fn reading_a_topic_nobody_published_reports_no_value() {
        let bus = registry();
        assert_eq!(
            unsafe { bus.read("brucebox.absent", std::ptr::null_mut(), 0) },
            -1
        );
    }

    #[test]
    fn a_short_buffer_only_reports_the_length_it_needs() {
        let bus = registry();
        bus.publish("brucebox.git", "repo", b"twelve bytes");

        let mut buffer = [0u8; 3];
        assert_eq!(
            unsafe { bus.read("repo", buffer.as_mut_ptr(), buffer.len()) },
            12
        );
        assert_eq!(
            &buffer, b"\0\0\0",
            "nothing is written when it does not fit"
        );
        assert_eq!(unsafe { bus.read("repo", std::ptr::null_mut(), 0) }, 12);
    }

    #[test]
    fn subscribers_hear_changes_and_not_repeats_or_their_own_publications() {
        let bus = registry();
        let recorder = Recorder {
            seen: StdMutex::new(Vec::new()),
        };
        let sink = &recorder as *const Recorder as *mut c_void;
        let subscription = bus
            .subscribe("repo", record, sink)
            .expect("the topic is valid");
        assert!(subscription > 0);

        bus.publish("brucebox.git", "repo", b"first");
        bus.publish("brucebox.git", "repo", b"first");
        bus.publish("brucebox.other", "repo", b"second");
        bus.publish("brucebox.git", "repo", b"first");

        let seen = recorder.seen.lock().unwrap();
        assert_eq!(
            seen.as_slice(),
            [
                ("repo".to_owned(), b"first".to_vec()),
                ("repo".to_owned(), b"second".to_vec()),
                ("repo".to_owned(), b"first".to_vec()),
            ],
            "only changes are announced"
        );
    }

    #[test]
    fn unsubscribing_stops_delivery() {
        let bus = registry();
        let recorder = Recorder {
            seen: StdMutex::new(Vec::new()),
        };
        let sink = &recorder as *const Recorder as *mut c_void;
        let id = bus.subscribe("repo", record, sink).unwrap();

        bus.publish("brucebox.git", "repo", b"before");
        bus.unsubscribe(id);
        bus.publish("brucebox.git", "repo", b"after");

        assert_eq!(recorder.seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_topic_must_look_like_a_contract_topic() {
        let bus = registry();

        assert_eq!(
            bus.publish("brucebox.git", "", b"x"),
            ContractStatus::BadTopic
        );
        assert_eq!(
            bus.publish("brucebox.git", "not a topic", b"x"),
            ContractStatus::BadTopic
        );
        assert_eq!(
            bus.publish(
                "brucebox.git",
                "repo",
                &vec![0u8; CONTRACT_PAYLOAD_LIMIT + 1]
            ),
            ContractStatus::TooLarge
        );
    }

    #[test]
    fn a_plugin_link_wires_the_registry_to_the_plugins_id() {
        let link = PluginBusLink::new("brucebox.git");
        let context = link.host_context();

        assert_eq!(context.api_version, brucebox_plugin_api::API_VERSION);
        assert_eq!(link.plugin_id(), "brucebox.git");
    }
}

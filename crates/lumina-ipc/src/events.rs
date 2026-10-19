//! Unsolicited event push.
//!
//! The orchestrator emits events such as `media_changed`; every connected
//! client receives them on the same stream as responses. Slow clients are
//! dropped from the broadcast list rather than blocking the orchestrator: a
//! client that cannot keep up must reconnect.

use std::fmt;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::framing::encode_line;
use crate::protocol::{MediaSnapshot, RgbSnapshot};

/// The five event kinds from the protocol specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The active player, status, or metadata changed.
    MediaChanged,
    /// The island entered a new state.
    StateChanged,
    /// The RGB engine or palette changed.
    RgbChanged,
    /// A configuration reload completed successfully.
    ConfigReloaded,
    /// A configuration reload failed validation.
    ConfigError,
}

impl EventKind {
    /// The exact spelling used on the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MediaChanged => "media_changed",
            Self::StateChanged => "state_changed",
            Self::RgbChanged => "rgb_changed",
            Self::ConfigReloaded => "config_reloaded",
            Self::ConfigError => "config_error",
        }
    }

    /// Every event kind, for documentation tests.
    pub const ALL: [Self; 5] = [
        Self::MediaChanged,
        Self::StateChanged,
        Self::RgbChanged,
        Self::ConfigReloaded,
        Self::ConfigError,
    ];
}

impl fmt::Display for EventKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One event line pushed to clients.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Which event fired.
    pub event: EventKind,
    /// Event-specific payload.
    pub data: Value,
}

impl Event {
    /// The island changed state.
    pub fn state_changed(state: impl Into<String>) -> Self {
        Self {
            event: EventKind::StateChanged,
            data: serde_json::json!({ "state": state.into() }),
        }
    }

    /// The active media session changed.
    pub fn media_changed(snapshot: &MediaSnapshot) -> Self {
        Self {
            event: EventKind::MediaChanged,
            data: serde_json::json!({ "media": snapshot }),
        }
    }

    /// The RGB engine or palette changed.
    pub fn rgb_changed(snapshot: &RgbSnapshot) -> Self {
        Self {
            event: EventKind::RgbChanged,
            data: serde_json::json!({ "rgb": snapshot }),
        }
    }

    /// A reload completed.
    pub fn config_reloaded(profile: impl Into<String>) -> Self {
        Self {
            event: EventKind::ConfigReloaded,
            data: serde_json::json!({ "profile": profile.into() }),
        }
    }

    /// A reload failed validation.
    pub fn config_error(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            event: EventKind::ConfigError,
            data: serde_json::json!({
                "key": key.into(),
                "message": message.into(),
            }),
        }
    }
}

/// Events a client may fall behind before it is unsubscribed.
const EVENT_BUFFER: usize = 256;

#[derive(Debug, Default)]
struct Inner {
    next_id: u64,
    subscribers: Vec<(u64, SyncSender<Vec<u8>>)>,
}

/// Fan-out handle held by the orchestrator and by every client thread.
#[derive(Clone, Debug, Default)]
pub struct EventSender {
    inner: Arc<Mutex<Inner>>,
}

impl EventSender {
    /// Create an empty bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Serialize and broadcast an event.
    ///
    /// Serialization of the event payload is the only fallible step; delivery
    /// to a disconnected client is silently skipped.
    pub fn emit(&self, event: &Event) -> Result<(), serde_json::Error> {
        let line = encode_line(event)?;
        self.emit_line(&line);
        Ok(())
    }

    /// Broadcast an already encoded line.
    ///
    /// A subscriber that has not drained its queue is unsubscribed instead of
    /// blocking the orchestrator. That client must reconnect to receive events
    /// again.
    pub fn emit_line(&self, line: &[u8]) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner
            .subscribers
            .retain(|(_, sender)| sender.try_send(line.to_vec()).is_ok());
    }

    /// Register a client.
    pub fn subscribe(&self) -> Subscription {
        let (sender, receiver) = mpsc::sync_channel(EVENT_BUFFER);
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let id = inner.next_id;
        inner.next_id += 1;
        inner.subscribers.push((id, sender));
        Subscription {
            id,
            receiver,
            bus: self.clone(),
        }
    }

    fn unsubscribe(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner
            .subscribers
            .retain(|(subscriber, _)| *subscriber != id);
    }

    /// Number of clients currently subscribed.
    pub fn subscriber_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .subscribers
            .len()
    }
}

/// A client's event feed. Dropping it unsubscribes.
#[derive(Debug)]
pub struct Subscription {
    id: u64,
    receiver: Receiver<Vec<u8>>,
    bus: EventSender,
}

impl Subscription {
    /// Receive the next event line, blocking until one arrives.
    pub fn recv(&self) -> Option<Vec<u8>> {
        self.receiver.recv().ok()
    }

    /// Receive with a timeout; used by client threads to notice shutdown.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Vec<u8>, RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.bus.unsubscribe(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_serializes_with_its_wire_name() {
        let events = [
            Event::state_changed("expanded"),
            Event::media_changed(&MediaSnapshot::default()),
            Event::rgb_changed(&RgbSnapshot::default()),
            Event::config_reloaded("work"),
            Event::config_error("widgets.clock", "bad value"),
        ];
        let names = [
            "state_changed",
            "media_changed",
            "rgb_changed",
            "config_reloaded",
            "config_error",
        ];
        for (event, name) in events.iter().zip(names) {
            let value = serde_json::to_value(event).unwrap();
            assert_eq!(value["event"], name);
            assert!(value["data"].is_object());
        }
    }

    #[test]
    fn every_kind_has_a_stable_name() {
        for kind in EventKind::ALL {
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{}\"", kind.as_str())
            );
        }
    }

    #[test]
    fn subscribers_receive_events_in_order() {
        let bus = EventSender::new();
        let subscription = bus.subscribe();
        bus.emit(&Event::state_changed("hover")).unwrap();
        bus.emit(&Event::state_changed("clock")).unwrap();
        let first = subscription
            .recv_timeout(Duration::from_millis(50))
            .unwrap();
        let second = subscription
            .recv_timeout(Duration::from_millis(50))
            .unwrap();
        assert!(String::from_utf8(first).unwrap().contains("hover"));
        assert!(String::from_utf8(second).unwrap().contains("clock"));
    }

    #[test]
    fn dropping_a_subscription_removes_it() {
        let bus = EventSender::new();
        {
            let _subscription = bus.subscribe();
            assert_eq!(bus.subscriber_count(), 1);
        }
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn emitting_without_subscribers_is_harmless() {
        let bus = EventSender::new();
        bus.emit(&Event::state_changed("collapsed")).unwrap();
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn a_subscriber_that_falls_behind_is_unsubscribed() {
        let bus = EventSender::new();
        let subscription = bus.subscribe();
        for index in 0..=EVENT_BUFFER {
            bus.emit(&Event::state_changed(index.to_string())).unwrap();
        }
        assert_eq!(bus.subscriber_count(), 0);
        assert!(
            subscription.recv_timeout(Duration::from_millis(10)).is_ok(),
            "buffered events remain readable after unsubscription"
        );
    }

    #[test]
    fn event_data_carries_the_documented_fields() {
        let state = serde_json::to_value(Event::state_changed("expanded")).unwrap();
        assert_eq!(state["data"]["state"], "expanded");
        let error = serde_json::to_value(Event::config_error("a.b", "bad")).unwrap();
        assert_eq!(error["data"]["key"], "a.b");
        assert_eq!(error["data"]["message"], "bad");
    }
}

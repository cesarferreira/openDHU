//! In-process event bus behind `GET /v1/events`.

use std::sync::atomic::{AtomicU64, Ordering};

use opendhu_core::{iso8601_utc, unix_now};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::api::Event;

/// Slow subscribers lose the oldest events and are told so (`stream.lagged`) rather than being
/// allowed to block the daemon.
const CAPACITY: usize = 256;

pub struct EventBus {
    tx: broadcast::Sender<Event>,
    next_id: AtomicU64,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            tx: broadcast::channel(CAPACITY).0,
            next_id: AtomicU64::new(1),
        }
    }

    pub fn emit(&self, kind: &str, session_id: Option<&str>, data: Value) {
        let event = Event {
            id: self.next_id.fetch_add(1, Ordering::SeqCst),
            kind: kind.into(),
            timestamp: iso8601_utc(unix_now()),
            session_id: session_id.map(str::to_string),
            data,
        };
        // No subscribers is normal.
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ids_increase_and_subscribers_see_events_in_order() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.emit("a", Some("s1"), json!({"n": 1}));
        bus.emit("b", None, json!({}));
        let (a, b) = (rx.try_recv().unwrap(), rx.try_recv().unwrap());
        assert_eq!(
            (a.kind.as_str(), a.id, a.session_id.as_deref()),
            ("a", 1, Some("s1"))
        );
        assert_eq!((b.kind.as_str(), b.id), ("b", 2));
        assert!(a.timestamp.ends_with('Z') && a.timestamp.len() == 20);
    }

    #[test]
    fn emitting_with_no_subscribers_is_fine() {
        EventBus::new().emit("x", None, json!(null));
    }

    #[test]
    fn a_slow_subscriber_is_told_it_lagged_instead_of_blocking_the_sender() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        for _ in 0..(CAPACITY + 10) {
            bus.emit("x", None, json!({}));
        }
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(_))
        ));
    }
}

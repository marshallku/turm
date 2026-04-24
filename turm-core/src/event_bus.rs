use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError, channel, sync_channel};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_SUBSCRIBER_BUFFER: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub kind: String,
    pub source: String,
    pub timestamp_ms: u64,
    pub payload: Value,
}

impl Event {
    pub fn new(kind: impl Into<String>, source: impl Into<String>, payload: Value) -> Self {
        Self {
            kind: kind.into(),
            source: source.into(),
            timestamp_ms: now_millis(),
            payload,
        }
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub struct EventReceiver {
    inner: Receiver<Event>,
}

impl EventReceiver {
    pub fn try_recv(&self) -> Option<Event> {
        self.inner.try_recv().ok()
    }

    pub fn recv(&self) -> Option<Event> {
        self.inner.recv().ok()
    }
}

enum SubscriberSender {
    Bounded(SyncSender<Event>),
    Unbounded(Sender<Event>),
}

impl SubscriberSender {
    fn deliver(&self, event: Event) -> DeliveryResult {
        match self {
            Self::Bounded(tx) => match tx.try_send(event) {
                Ok(()) => DeliveryResult::Ok,
                Err(TrySendError::Full(_)) => DeliveryResult::Full,
                Err(TrySendError::Disconnected(_)) => DeliveryResult::Disconnected,
            },
            Self::Unbounded(tx) => match tx.send(event) {
                Ok(()) => DeliveryResult::Ok,
                Err(_) => DeliveryResult::Disconnected,
            },
        }
    }
}

enum DeliveryResult {
    Ok,
    Full,
    Disconnected,
}

struct Subscriber {
    pattern: String,
    sender: SubscriberSender,
}

pub struct EventBus {
    subscribers: Mutex<Vec<Subscriber>>,
    default_buffer: usize,
}

impl EventBus {
    pub fn new() -> Self {
        Self::with_default_buffer(DEFAULT_SUBSCRIBER_BUFFER)
    }

    pub fn with_default_buffer(default_buffer: usize) -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
            default_buffer,
        }
    }

    pub fn subscribe(&self, pattern: impl Into<String>) -> EventReceiver {
        self.subscribe_with_buffer(pattern, self.default_buffer)
    }

    pub fn subscribe_with_buffer(
        &self,
        pattern: impl Into<String>,
        buffer: usize,
    ) -> EventReceiver {
        let (tx, rx) = sync_channel(buffer);
        self.subscribers.lock().unwrap().push(Subscriber {
            pattern: pattern.into(),
            sender: SubscriberSender::Bounded(tx),
        });
        EventReceiver { inner: rx }
    }

    /// Subscribe with an unbounded channel. Use this for external wire streams
    /// (e.g. the socket `event.subscribe` projection) where event loss would
    /// violate the client contract. The caller is responsible for draining.
    pub fn subscribe_unbounded(&self, pattern: impl Into<String>) -> EventReceiver {
        let (tx, rx) = channel();
        self.subscribers.lock().unwrap().push(Subscriber {
            pattern: pattern.into(),
            sender: SubscriberSender::Unbounded(tx),
        });
        EventReceiver { inner: rx }
    }

    pub fn publish(&self, event: Event) {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|sub| {
            if !pattern_matches(&sub.pattern, &event.kind) {
                return true;
            }
            match sub.sender.deliver(event.clone()) {
                DeliveryResult::Ok => true,
                DeliveryResult::Full => {
                    log::warn!(
                        "event bus subscriber pattern={:?} buffer full, dropping kind={:?}",
                        sub.pattern,
                        event.kind
                    );
                    true
                }
                DeliveryResult::Disconnected => false,
            }
        });
    }

    pub fn subscriber_count(&self) -> usize {
        self.subscribers.lock().unwrap().len()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

fn pattern_matches(pattern: &str, kind: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(".*") {
        return kind.len() > prefix.len()
            && kind.starts_with(prefix)
            && kind.as_bytes()[prefix.len()] == b'.';
    }
    pattern == kind
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mk(kind: &str) -> Event {
        Event::new(kind, "test", json!({}))
    }

    #[test]
    fn pattern_exact_match() {
        assert!(pattern_matches("foo.bar", "foo.bar"));
        assert!(!pattern_matches("foo.bar", "foo.baz"));
        assert!(!pattern_matches("foo.bar", "foo"));
    }

    #[test]
    fn pattern_star_matches_anything() {
        assert!(pattern_matches("*", "anything.at.all"));
        assert!(pattern_matches("*", "x"));
    }

    #[test]
    fn pattern_prefix_wildcard() {
        assert!(pattern_matches("foo.*", "foo.bar"));
        assert!(pattern_matches("foo.*", "foo.bar.baz"));
        assert!(!pattern_matches("foo.*", "foo"));
        assert!(!pattern_matches("foo.*", "foobar"));
        assert!(!pattern_matches("foo.*", "bar.foo"));
    }

    #[test]
    fn publish_delivers_to_matching_subscriber() {
        let bus = EventBus::new();
        let rx = bus.subscribe("calendar.*");
        bus.publish(mk("calendar.event_imminent"));
        let e = rx.try_recv().expect("matching event should arrive");
        assert_eq!(e.kind, "calendar.event_imminent");
    }

    #[test]
    fn publish_skips_non_matching_subscriber() {
        let bus = EventBus::new();
        let rx = bus.subscribe("slack.*");
        bus.publish(mk("calendar.event_imminent"));
        assert!(rx.try_recv().is_none());
    }

    #[test]
    fn multiple_subscribers_all_receive() {
        let bus = EventBus::new();
        let rx_all = bus.subscribe("*");
        let rx_foo = bus.subscribe("foo.*");
        let rx_bar = bus.subscribe("bar.*");
        bus.publish(mk("foo.created"));
        assert_eq!(rx_all.try_recv().unwrap().kind, "foo.created");
        assert_eq!(rx_foo.try_recv().unwrap().kind, "foo.created");
        assert!(rx_bar.try_recv().is_none());
    }

    #[test]
    fn full_subscriber_drops_newest_and_preserves_queued() {
        let bus = EventBus::new();
        let rx = bus.subscribe_with_buffer("*", 2);
        bus.publish(mk("a"));
        bus.publish(mk("b"));
        bus.publish(mk("c"));
        assert_eq!(rx.try_recv().unwrap().kind, "a");
        assert_eq!(rx.try_recv().unwrap().kind, "b");
        assert!(rx.try_recv().is_none());
    }

    #[test]
    fn dropped_receiver_is_cleaned_up_on_next_publish() {
        let bus = EventBus::new();
        let rx = bus.subscribe("*");
        bus.publish(mk("first"));
        assert_eq!(bus.subscriber_count(), 1);
        drop(rx);
        bus.publish(mk("second"));
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn unbounded_subscriber_never_drops() {
        let bus = EventBus::new();
        let rx = bus.subscribe_unbounded("*");
        for i in 0..1000 {
            bus.publish(mk(&format!("k{i}")));
        }
        for i in 0..1000 {
            assert_eq!(rx.try_recv().unwrap().kind, format!("k{i}"));
        }
        assert!(rx.try_recv().is_none());
    }

    #[test]
    fn unbounded_and_bounded_coexist() {
        let bus = EventBus::new();
        let rx_u = bus.subscribe_unbounded("*");
        let rx_b = bus.subscribe_with_buffer("*", 2);
        bus.publish(mk("a"));
        bus.publish(mk("b"));
        bus.publish(mk("c"));
        // Unbounded got all three; bounded kept only first two.
        assert_eq!(rx_u.try_recv().unwrap().kind, "a");
        assert_eq!(rx_u.try_recv().unwrap().kind, "b");
        assert_eq!(rx_u.try_recv().unwrap().kind, "c");
        assert!(rx_u.try_recv().is_none());
        assert_eq!(rx_b.try_recv().unwrap().kind, "a");
        assert_eq!(rx_b.try_recv().unwrap().kind, "b");
        assert!(rx_b.try_recv().is_none());
    }

    #[test]
    fn event_timestamp_is_populated() {
        let before = now_millis();
        let e = Event::new("x", "y", json!({}));
        let after = now_millis();
        assert!(e.timestamp_ms >= before && e.timestamp_ms <= after);
    }
}

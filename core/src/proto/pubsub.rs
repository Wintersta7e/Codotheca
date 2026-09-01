//! Per-subscription backpressure.
//!
//! On overflow the queue for a topic is replaced *atomically* by one snapshot, so no delta
//! older than the snapshot can arrive after it. The consumer detects gaps by sequence number
//! and asks for a fresh snapshot; a stale delta is dropped rather than reconciled.

use crate::proto::transport::FrameSink;
use crate::proto::wire::{Epoch, Outbound};
use crate::protocol::Topic;
use serde::de::IntoDeserializer as _;
use serde::Deserialize as _;
use serde_json::Value;
use std::collections::VecDeque;
use std::io::Write as _;

/// Queued deltas for one topic before the backlog is replaced by a snapshot.
pub const TOPIC_HIGH_WATER: usize = 256;

/// One frame waiting for the writer.
#[derive(Debug, Clone, PartialEq)]
pub enum Queued {
    Delta {
        seq: u64,
        event: String,
        data: Value,
    },
    Snapshot {
        through_seq: u64,
        data: Value,
    },
}

/// One topic's outbound queue.
#[derive(Debug)]
pub struct TopicQueue {
    high_water: usize,
    next_seq: u64,
    pending: VecDeque<Queued>,
}

impl TopicQueue {
    #[must_use]
    pub fn new(high_water: usize) -> Self {
        Self {
            high_water,
            next_seq: 1,
            pending: VecDeque::new(),
        }
    }

    /// Enqueues one delta and returns the sequence number it was given.
    pub fn push(&mut self, event: &str, data: Value) -> u64 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.pending.push_back(Queued::Delta {
            seq,
            event: event.to_owned(),
            data,
        });
        seq
    }

    #[must_use]
    pub fn over_high_water(&self) -> bool {
        self.pending.len() > self.high_water
    }

    /// Replaces the entire queue with one snapshot. Nothing already queued survives.
    pub fn replace_with_snapshot(&mut self, data: Value) {
        let through_seq = self.through_seq();
        self.pending.clear();
        self.pending
            .push_back(Queued::Snapshot { through_seq, data });
    }

    #[must_use]
    pub fn through_seq(&self) -> u64 {
        self.next_seq.saturating_sub(1)
    }

    #[must_use]
    pub fn peek(&self) -> Option<&Queued> {
        self.pending.front()
    }

    pub fn pop(&mut self) -> Option<Queued> {
        self.pending.pop_front()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// What `publish` wants the caller to do next.
#[derive(Debug, PartialEq, Eq)]
pub enum Published {
    Sent {
        seq: u64,
    },
    /// The queue passed its high-water mark. Compute a snapshot — outside any transaction —
    /// and hand it back with `supply_snapshot`.
    NeedsSnapshot {
        through_seq: u64,
    },
    /// Nobody is subscribed to this topic.
    Unsubscribed,
}

#[derive(Debug)]
struct TopicState {
    topic: Topic,
    queue: TopicQueue,
}

/// Owns one outbound queue per subscribed topic and the only `FrameSink` events travel on.
#[derive(Debug)]
pub struct Publisher {
    sink: Option<FrameSink>,
    epoch: Epoch,
    high_water: usize,
    topics: Vec<TopicState>,
}

impl Publisher {
    #[must_use]
    pub fn new(sink: FrameSink, epoch: Epoch, high_water: usize) -> Self {
        Self {
            sink: Some(sink),
            epoch,
            high_water,
            topics: Vec::new(),
        }
    }

    pub fn subscribe(&mut self, topic: Topic) {
        if !self.topics.iter().any(|t| t.topic == topic) {
            self.topics.push(TopicState {
                topic,
                queue: TopicQueue::new(self.high_water),
            });
        }
    }

    pub fn unsubscribe(&mut self, topic: Topic) {
        self.topics.retain(|t| t.topic != topic);
    }

    fn state(&mut self, topic: Topic) -> Option<&mut TopicState> {
        self.topics.iter_mut().find(|t| t.topic == topic)
    }

    pub fn publish(&mut self, topic: Topic, event: &str, data: Value) -> Published {
        let epoch = self.epoch;
        let Some(state) = self.state(topic) else {
            return Published::Unsubscribed;
        };
        let seq = state.queue.push(event, data);
        let over = state.queue.over_high_water();
        if let Some(sink) = self.sink.as_ref() {
            Self::flush_one(sink, epoch, topic, &mut self.topics);
        }
        if over {
            Published::NeedsSnapshot { through_seq: seq }
        } else {
            Published::Sent { seq }
        }
    }

    /// Replaces everything queued for `topic` with one snapshot, atomically.
    pub fn supply_snapshot(&mut self, topic: Topic, data: Value) {
        let epoch = self.epoch;
        if let Some(state) = self.state(topic) {
            state.queue.replace_with_snapshot(data);
        }
        if let Some(sink) = self.sink.as_ref() {
            Self::flush_one(sink, epoch, topic, &mut self.topics);
        }
    }

    /// The consumer detected a gap and asked for a fresh snapshot. Returns the sequence the
    /// snapshot must be stamped through, or `None` if nobody is subscribed.
    pub fn resync(&mut self, topic: Topic) -> Option<u64> {
        // A closure, not `map(TopicState::through_seq)`: `state` yields `&mut TopicState` and a
        // function item taking `&TopicState` will not coerce there (E0631).
        self.state(topic).map(|s| s.queue.through_seq())
    }

    /// Drains whatever the writer queue will now accept, for every topic.
    pub fn flush(&mut self) {
        let Some(sink) = self.sink.as_ref() else {
            return;
        };
        let epoch = self.epoch;
        let topics: Vec<Topic> = self.topics.iter().map(|t| t.topic).collect();
        for topic in topics {
            Self::flush_one(sink, epoch, topic, &mut self.topics);
        }
    }

    /// Releases the transport sender once all producers have shut down.
    pub(crate) fn close(&mut self) {
        self.sink = None;
    }

    fn flush_one(sink: &FrameSink, epoch: Epoch, topic: Topic, topics: &mut [TopicState]) {
        let Some(state) = topics.iter_mut().find(|t| t.topic == topic) else {
            return;
        };
        while let Some(head) = state.queue.peek().cloned() {
            let frame = match head {
                Queued::Delta { seq, event, data } => Outbound::Event {
                    epoch,
                    topic,
                    event,
                    seq,
                    data,
                },
                Queued::Snapshot { through_seq, data } => Outbound::Snapshot {
                    epoch,
                    topic,
                    through_seq,
                    data,
                },
            };
            if sink.try_send(&frame).is_err() {
                return;
            }
            let _ = state.queue.pop();
        }
    }
}

/// The publish seam every background job, art render and session tick emits through (**R16**).
///
/// Narrow on purpose: a `&str` topic and a `&str` event name, so a worker thread needs neither
/// the generated `Topic` enum nor a `&mut Publisher`. `&self`, because holders share it as
/// `Arc<dyn EventSink>`.
pub trait EventSink: Send + Sync {
    fn emit(&self, topic: &str, event: &str, payload: Value);
}

/// The `EventSink` this plan ships: a `Publisher` behind a mutex.
///
/// `Publisher::publish` takes `&mut self` and `emit` takes `&self`, so this cannot be an
/// `impl EventSink for Publisher` — the mutex is what makes the two signatures meet.
#[derive(Debug)]
pub struct PublisherSink {
    inner: std::sync::Mutex<Publisher>,
    wants_snapshot: std::sync::Mutex<Vec<Topic>>,
}

impl PublisherSink {
    #[must_use]
    pub fn new(publisher: Publisher) -> Self {
        Self {
            inner: std::sync::Mutex::new(publisher),
            wants_snapshot: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Drives the `Publisher` directly — `subscribe`, `supply_snapshot`, `flush` — under the
    /// same lock `emit` takes.
    pub fn with<R>(&self, f: impl FnOnce(&mut Publisher) -> R) -> R {
        f(&mut Self::lock(&self.inner))
    }

    /// Topics whose queue passed the high-water mark while emitting, taken and cleared. Only
    /// the owner of the `CommandHandler` can compute a snapshot, so only it can answer these;
    /// see "Seams this plan leaves open".
    pub fn take_snapshot_requests(&self) -> Vec<Topic> {
        std::mem::take(&mut Self::lock(&self.wants_snapshot))
    }

    /// A poisoned mutex means another thread panicked mid-`emit`. The queue is still
    /// structurally sound, and silently dropping every later event is worse than continuing,
    /// so the guard is taken either way.
    fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl EventSink for PublisherSink {
    fn emit(&self, topic: &str, event: &str, payload: Value) {
        // The wire spelling of `Topic` is its serde rename, so the generated enum is the only
        // place the four topic names are written down.
        let de: serde::de::value::StrDeserializer<'_, serde::de::value::Error> =
            topic.into_deserializer();
        let Ok(topic) = Topic::deserialize(de) else {
            let _ = writeln!(
                std::io::stderr(),
                "codotheca-core: dropped {event} on unknown topic {topic:?}"
            );
            return;
        };
        let mut publisher = Self::lock(&self.inner);
        if let Published::NeedsSnapshot { .. } = publisher.publish(topic, event, payload) {
            Self::lock(&self.wants_snapshot).push(topic);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{EventSink, Published, Publisher, PublisherSink, Queued, TopicQueue};
    use crate::proto::transport::Transport;
    use crate::proto::wire::{Epoch, Outbound};
    use crate::protocol::Topic;
    use serde_json::json;

    struct Collect(std::sync::mpsc::Sender<Vec<u8>>);

    impl std::io::Write for Collect {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            let _ = self.0.send(b.to_vec());
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Blocks every write until the test permits one. Dropping the permit sender releases it.
    struct Gate(std::sync::mpsc::Receiver<()>);

    impl std::io::Write for Gate {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            let _ = self.0.recv();
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn overflow_replaces_the_queue_with_one_snapshot_and_later_deltas_follow_it() {
        let mut q = TopicQueue::new(3);
        for _ in 0..4 {
            q.push("upserted", json!({}));
        }
        assert!(q.over_high_water());
        q.replace_with_snapshot(json!({ "projects": [] }));
        assert_eq!(q.len(), 1, "the whole backlog is gone, not merely trimmed");
        let head = q.pop().expect("snapshot");
        let Queued::Snapshot { through_seq, .. } = head else {
            panic!("expected a snapshot")
        };
        assert_eq!(through_seq, 4);
        let seq = q.push("upserted", json!({}));
        assert!(
            seq > through_seq,
            "a delta after a snapshot must carry seq > through_seq"
        );
    }

    #[test]
    fn publishing_to_an_unsubscribed_topic_queues_nothing() {
        let t = Transport::start(std::io::empty(), std::io::sink(), 8).expect("transport");
        let mut p = Publisher::new(t.sink.clone(), Epoch(1), 4);
        assert_eq!(
            p.publish(Topic::Scan, "progress", json!({})),
            Published::Unsubscribed
        );
        drop(p);
        t.join();
    }

    #[test]
    fn a_stalled_writer_makes_the_topic_ask_for_a_snapshot() {
        let (permit, gate) = std::sync::mpsc::channel::<()>();
        let t = Transport::start(std::io::empty(), Gate(gate), 1).expect("transport");
        let mut p = Publisher::new(t.sink.clone(), Epoch(1), 2);
        p.subscribe(Topic::Scan);
        let mut asked = None;
        for _ in 0..64 {
            if let Published::NeedsSnapshot { through_seq } =
                p.publish(Topic::Scan, "repo_found", json!({}))
            {
                asked = Some(through_seq);
                break;
            }
        }
        assert!(
            asked.is_some(),
            "a stalled writer must reach the high-water mark"
        );
        drop(p);
        drop(permit);
        t.join();
    }

    #[test]
    fn a_snapshot_is_addressed_to_the_epoch_that_asked_for_it() {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let t = Transport::start(std::io::empty(), Collect(tx), 8).expect("transport");
        let mut p = Publisher::new(t.sink.clone(), Epoch(9), 4);
        p.subscribe(Topic::Projects);
        p.supply_snapshot(Topic::Projects, json!({ "projects": [] }));
        let _header = rx.recv().expect("header");
        let body = rx.recv().expect("body");
        let frame: Outbound = serde_json::from_slice(&body).expect("decode");
        match frame {
            Outbound::Snapshot { epoch, topic, .. } => {
                assert_eq!(epoch, Epoch(9));
                assert_eq!(topic, Topic::Projects);
            }
            other => panic!("expected a snapshot, got {other:?}"),
        }
        drop(p);
        t.join();
    }

    // R16: the two below are the whole test for `EventSink`. The first proves the trait is
    // usable the way plans 09, 10 and 11b use it — an `Arc<dyn EventSink>` on a worker thread —
    // and that what comes out is an ordinary `Outbound::Event` on the real transport.
    #[test]
    fn an_event_sink_emits_through_the_publisher_from_another_thread() {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let t = Transport::start(std::io::empty(), Collect(tx), 8).expect("transport");
        let mut p = Publisher::new(t.sink.clone(), Epoch(3), 4);
        p.subscribe(Topic::Scan);
        let sink: std::sync::Arc<dyn EventSink> = std::sync::Arc::new(PublisherSink::new(p));
        let worker = {
            let sink = std::sync::Arc::clone(&sink);
            std::thread::spawn(move || sink.emit("scan", "progress", json!({ "done": 1 })))
        };
        worker.join().expect("worker");
        let _header = rx.recv().expect("header");
        let body = rx.recv().expect("body");
        match serde_json::from_slice::<Outbound>(&body).expect("decode") {
            Outbound::Event {
                epoch,
                topic,
                event,
                seq,
                ..
            } => {
                assert_eq!(epoch, Epoch(3));
                assert_eq!(topic, Topic::Scan);
                assert_eq!(event, "progress");
                assert_eq!(seq, 1);
            }
            other => panic!("expected an event, got {other:?}"),
        }
        drop(sink);
        t.join();
    }

    #[test]
    fn an_unknown_topic_name_is_dropped_rather_than_panicking() {
        let t = Transport::start(std::io::empty(), std::io::sink(), 8).expect("transport");
        let sink = PublisherSink::new(Publisher::new(t.sink.clone(), Epoch(1), 4));
        sink.emit("nonesuch", "progress", json!({}));
        assert!(sink.take_snapshot_requests().is_empty());
        drop(sink);
        t.join();
    }
}

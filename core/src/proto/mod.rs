//! The core half of the sidecar protocol.

pub mod frame;
pub mod pubsub;
pub mod transport;
pub mod txguard;
pub mod wire;

pub use pubsub::{EventSink, PublisherSink};

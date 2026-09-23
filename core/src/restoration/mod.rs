//! §34 — restoration's core half: **`health_delta` gets its producer.**
//!
//! A delta is a transition in the open debt set (A15) and nothing else — never a completion tick.
//! The producer takes its own two snapshots inside the writer's transaction — [`LayerValues::read`]
//! before §28's debt write, again after it — diffs them under §30.1's precondition, writes one row
//! per changed layer, and hands back the event to announce once the transaction has committed.
//! One transaction, no second connection, and no third stored quantity: a delta is a function of
//! two observed readings.
//!
//! **Every writer of the open debt set calls it, in its own transaction**, and there are four:
//! `JobRunner::settle` around §28's singleton evaluator, J7's item build (which writes inside the
//! job's own transaction because only it holds the HEAD enumeration), `SyncRunner::settle`
//! around the same evaluator for a `ProjectRemote` sync, and §32's advisory item sync at an
//! advisory sweep's close (`advisories::items::settle_advisory_items`, per project, inside the
//! runner's transaction for that close). A producer wired at fewer sites is the
//! trait-with-a-fake-and-no-real-caller shape by another route: it compiles, every unit test is
//! green, and the TODO the user just closed plays nothing.
//!
//! The event is **only** the surge's transport. The rendered state refreshes off
//! `projects.upserted` (R121) whether or not this event is sent, so the page never depends on an
//! animation's delivery to show the truth.

pub mod delta;
pub mod value;

pub use delta::{
    detected_in_for, emit_health_delta, emittable_layers, health_delta_event, record_after_write,
    record_layer_deltas,
};
pub use value::{LayerTransition, LayerValues};

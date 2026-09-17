//! §21 — remote sync, the runner.
//!
//! **Sync is its own runner and is not a job** (§21.1). It adds no `JobKind` variant, writes no
//! `project_job_state` row and does not ride `JobOutcome::Partial`, which stays declared, unused
//! and untouched by phase 2 (A2). Six structural blockers put it here rather than in the job
//! queue, and the two that decide it are the shape of the work and the shape of the budget: a
//! sync task has no `location_id` and the queue coalesces on one, and no admission concept in
//! `core::jobs` can express *a budget shared across every task, refilling at a wall-clock instant
//! the server names*.
//!
//! One dedicated blocking thread carrying **one HTTP request in flight at a time**, assembled as
//! `SyncPump` beside `JobPump` with the same cancel-then-join stop discipline.
//!
//! **This module declares no HTTP client and adds no dependency** (R66). `core::http` is p2-20's;
//! what lives here is `http::ObservingTransport`, the decorator that mirrors `x-ratelimit-*` from
//! **every** response — error responses included — and runs §21.8's classification.

pub mod classify;
pub mod http;
pub mod outcome;
pub mod state;
pub mod store;
pub mod task;

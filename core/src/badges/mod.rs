//! §40's badges: eight fixed predicates, each latched once in `badge_award`, paying nothing and
//! unlocking nothing.
//!
//! The table's readers and writers are closed (§40.7): this module (evaluate, list, `due`, mark
//! landed), the sidecar registry in `index::sidecar` (export and restore, through
//! [`store::export_section`] and [`store::restore_section`]) and the merge's repoint of
//! `project_id`. Nothing else reads the table — that is what *unlocks nothing* means in code.
//! The schema holds the latch itself: an award is never deleted, never re-inserted and never
//! rewritten, and only `project_id` and a single landing ever move.

pub mod store;

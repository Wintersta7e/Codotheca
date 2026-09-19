//! §31 — per-project completion.
//!
//! **Ten per-check rows, never two integers.** `project_check` is the only owner of check state;
//! `project.completion_lit` / `project.completion_applicable` are a projection recomputed from
//! those rows in the same transaction, by one function with exactly one production call site.
//!
//! The module is split so that the part that decides is testable without a database:
//!
//! - [`evaluate`] is **pure** — a shaped input struct to ten rows, no database, no remote, and no
//!   clock beyond the `now` handed to it.
//! - `inputs` is the only file here that reads a table or names `crate::remote`.
//! - `store` owns `project_check`.
//! - [`proposal`] owns §31.4's archetype proposal, in both vocabularies.
//!
//! **Six of the ten checks read §28's stored answer and re-derive nothing** (R124). §28.10 left
//! the direction open — *"§31 owns the check side and reads the item, **or** owns the predicate
//! and §28 reads it, but never both"* — and R124 closed it toward §28, which owns the singleton
//! item evaluator. Two evaluations of one predicate is R12 with a user-visible disagreement at
//! the end of it: a tick reading `pass` beside an open item saying otherwise, each green in its
//! own tests.

pub mod proposal;

//! §56.4–§56.9: the user's exclusion list — §4.3's built-in thirty followed by the entries the
//! user adds — and everything that keeps a covered repository unread.
//!
//! `list` answers every coverage question, `gate` holds the core's one list behind a pointer
//! swap, and `store` owns the `scan_exclusion` rows, the dialog-path normaliser,
//! `location.excluded` and their sidecar section. `visibility` is the one SQL text that reads
//! `location.excluded` back to decide whether a project is on the shelf.

pub mod gate;
pub mod list;
pub mod store;
pub mod visibility;

//! §56.4–§56.9: the user's exclusion list — §4.3's built-in thirty followed by the entries the
//! user adds — and everything that keeps a covered repository unread.
//!
//! `store` owns the `scan_exclusion` rows and their sidecar section.

pub mod store;

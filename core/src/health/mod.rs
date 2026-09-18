//! §30 — a project's health as a **reading**.
//!
//! **Nothing about the reading is stored.** §30.11 rules *migration: none* and this module adds
//! none: the reading is derived at read time on `project_presence`'s precedent
//! (`crate::scan::presence::project_presence` — *"`project` has no presence column: this is
//! derived at read time, which is why the function is pure and takes a slice"*), the per-check
//! switches ride `app_meta`, which is key/value, and `acknowledged_at` has been a declared column
//! since `0001_meta_and_projects.sql:102`.
//!
//! The only writes this module adds anywhere are the `acknowledged_at` stamp and the switch keys.

pub mod acknowledge;
pub mod enrolment;
pub mod state;

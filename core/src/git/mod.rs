//! Git access. Every invocation is config-neutralised, argv only, never a shell (§3.2).
//!
//! Plan 05 adds the backend, the neutralised invocation and the process-tree kill. This module
//! currently carries only the version floor, which the shell mirrors and a test pins.

pub mod version;

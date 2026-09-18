//! §30.5 — enrolment, **derived and not stored**, in `project_presence`'s shape: one pure
//! function, the only expression of the gate, no column and no migration.

/// Whether the user has ever acknowledged this project.
///
/// **Phase 4 adds the Amnesty verdict here as a second input, inside this function** — not as a
/// replacement for it, and not as a second predicate somewhere else.
#[must_use]
pub const fn is_enrolled(acknowledged_at: Option<i64>) -> bool {
    acknowledged_at.is_some()
}

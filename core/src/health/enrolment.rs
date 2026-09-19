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

/// §30.5 — **the compute gate: §29's blob read, and only that.**
///
/// It does **not** gate the enumeration. J3 already enumerates every non-Reference project
/// unsuppressed in the shipped build (`core/src/git/inventory.rs`), so suppressing J7's would
/// suppress work that already runs; and it does not gate anything bounded. §32's six named
/// lockfiles and §29's three presence probes run for a suppressed project and are simply not
/// surfaced.
///
/// This exists as a separate name from [`surface_suppressed`] **although the expression is
/// identical**, because the word *suppression* was carrying two rules and a reader who sees one
/// name applies it to both. They differ in what they gate, not in whom.
///
/// This supersedes `D1-debt-item.md`'s *"Suppressed backlog projects are not swept either — the
/// suppression is a detection gate, not a render gate."* Under A11.2 that holds only for sources
/// gated here.
#[must_use]
pub const fn compute_suppressed(enrolled: bool, is_archived: bool) -> bool {
    !enrolled || is_archived
}

/// §30.5 — **the surface gate: rendering, ranking (§35) and notification, and nothing else.**
///
/// It gates **computation of no kind**. The settled row's purpose is a *surfacing* harm — a wall
/// of red marks across a shelf of forgotten repositories — and the compute gate exists only
/// because §29's tier is genuinely expensive.
#[must_use]
pub const fn surface_suppressed(enrolled: bool, is_archived: bool) -> bool {
    !enrolled || is_archived
}

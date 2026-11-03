//! §40.2's populations, as SQL over a `project` row aliased `p`, each built on the reveal's one
//! live population and never restating it.

use crate::stats::reveal::LIVE;

/// **H**, the history population: live, not Reference, not shallow. A Reference project's days
/// are not the user's history, and a shallow clone's first commit is a truncation point, not a
/// beginning.
pub(crate) fn h_sql() -> String {
    format!("({LIVE} AND p.is_reference = 0 AND p.is_shallow = 0)")
}

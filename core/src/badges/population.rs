//! §40.2's populations, as SQL over a `project` row aliased `p`, each built on the reveal's one
//! live population and never restating it.

use rusqlite::Connection;

use crate::health::enrolment::{is_enrolled, surface_suppressed};
use crate::index::IndexError;
use crate::projects::current::IS_CURRENT_SQL;
use crate::protocol::{ProjectId, RevealBasis};
use crate::stats::reveal::{self, LIVE};

/// **H**, the history population: live, not Reference, not shallow. A Reference project's days
/// are not the user's history, and a shallow clone's first commit is a truncation point, not a
/// beginning.
pub(crate) fn h_sql() -> String {
    format!("({LIVE} AND p.is_reference = 0 AND p.is_shallow = 0)")
}

/// **S**, the state population: live, not Reference. Reference is excluded from XP and from
/// every statistic, and a badge is one.
pub(crate) fn s_sql() -> String {
    format!("({LIVE} AND p.is_reference = 0)")
}

/// **P**, the present population: **S** with a present copy — not removed, and holding a location
/// that is not removed. A standing badge counts what the library holds now, so a removed project
/// leaves it.
pub(crate) fn p_sql() -> String {
    format!("({} AND {IS_CURRENT_SQL})", s_sql())
}

/// The basis §10.4's history figures carry — the projects J4 has covered of the live total, and
/// whether every project that could yield history has — read through the reveal's own producer,
/// so the badge surface draws *not computed* from the same population the reveal does.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub(crate) fn history_basis(conn: &Connection) -> Result<RevealBasis, IndexError> {
    let pop = reveal::population(conn)?;
    Ok(reveal::basis(pop.history_covered, &pop))
}

/// **L**, Lifeline's population: **P** less every project §30.5's surface gate suppresses — the
/// unacknowledged backlog and the archives, which are ruled outside maintenance. Read through
/// §30.5's own functions, never a second spelling of them.
///
/// **§44's retirement re-points this call** when it makes the gate's second input `retired`: both are
/// `bool`, so passing `is_archived` would still compile.
///
/// # Errors
/// Fails when SQLite refuses the read.
pub(crate) fn lifeline_population(conn: &Connection) -> Result<Vec<ProjectId>, IndexError> {
    let mut st = conn.prepare_cached(&format!(
        "SELECT p.id, p.acknowledged_at, p.is_archived FROM project p WHERE {} ORDER BY p.id",
        p_sql()
    ))?;
    let rows = st
        .query_map([], |r| {
            Ok((
                ProjectId(r.get(0)?),
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, i64>(2)? != 0,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .filter(|&(_, acknowledged_at, archived)| {
            !surface_suppressed(is_enrolled(acknowledged_at), archived)
        })
        .map(|(id, _, _)| id)
        .collect())
}

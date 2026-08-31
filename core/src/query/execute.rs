//! The executor R37 found missing. Term for term with `app/src/renderer/shelf/evaluate.ts`.
//!
//! **It filters in Rust, not in SQL, and the reason is three-valued logic.** §8.3's evaluator is
//! tri-state: a term whose input was never observed is *unknown*, and an unknown row is not a
//! match — but neither is it a *false*. SQL collapses the two: `WHERE is_dirty = 1` and
//! `WHERE is_dirty = 0` both exclude a location nobody has looked at, so `-is:dirty` would
//! return "every repository we have not observed" dressed as "every clean repository". That is
//! render-unknown-as-zero, on the wire, in the one place the user is asking about state.
//!
//! **The cost is affordable, and the page shape is why.** §8.2's response is sectioned: every
//! matched row is needed before any window is applied, because `EraAggregate` sums over the
//! whole matched set and `Window` pages *rendered* rows rather than `LIMIT/OFFSET`. A SQL
//! `WHERE` would save no I/O — the same rows are read either way. What SQL does earn is the
//! base predicate, a fixed shape with `idx_project_shelf_order` behind it, and that stays in
//! `projects::rows`.
//!
//! **What it costs is an estimate and is labelled one.** At §8.3's ~1,000 projects the three
//! statements in `load_project_rows` dominate and the filter is `rows × terms` comparisons over
//! memory; `probe-results.md` has no SQLite figure. If a measurement ever shows the load
//! dominating, the fix is not to move the filter into SQL — §8.3 already rules per-keystroke
//! filtering client-side over the projection, precisely so this command is not on that path.

use std::collections::{BTreeMap, BTreeSet};

use crate::art::compose::local_year;
use crate::projects::rows::LoadedRow;
use crate::protocol::{CollectionId, ConditionSignal, LocationKind, ProjectRow};
use crate::query::ast::{
    Cmp, HasAttribute, IgnoredReason, IgnoredTerm, IsFlag, QueryAst, QueryTerm, TextField,
};

const DAY: i64 = 86_400;

/// §8.0b's base predicate, stated once so a reader does not have to infer it from a `filter`.
pub const BASE_PREDICATE: &str = "A bare query returns no is_reference rows and no is_hidden \
rows (§8.0b). is:reference and is:hidden opt their own rows back in; a negated term does not.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermTruth {
    True,
    False,
    /// Never observed. Matched by no polarity — this is the invariant the whole module exists for.
    Unknown,
}

impl TermTruth {
    #[must_use]
    pub const fn from_bool(v: Option<bool>) -> Self {
        match v {
            Some(true) => Self::True,
            Some(false) => Self::False,
            None => Self::Unknown,
        }
    }

    const fn known(v: bool) -> Self {
        if v {
            Self::True
        } else {
            Self::False
        }
    }

    fn from_count(v: Option<u32>) -> Self {
        Self::from_bool(v.map(|n| n > 0))
    }
}

#[derive(Debug)]
pub struct ExecContext<'a> {
    /// unix seconds
    pub now: i64,
    pub tz_offset_min: i32,
    /// `app_meta.first_run_completed_at`; `None` makes `is:new` unknown, never false (§10.5a).
    pub first_run_completed_at: Option<i64>,
    pub collection_ids_by_name: &'a BTreeMap<String, i64>,
    pub paths_are_case_sensitive: bool,
    /// §8.3's one round-tripping term, filled from `fts_commits` by the caller.
    pub commit_subject_hits: Option<&'a BTreeSet<i64>>,
}

fn folded_contains(haystack: Option<&str>, needle: &str) -> bool {
    haystack.is_some_and(|h| h.to_lowercase().contains(&needle.to_lowercase()))
}

fn bare_truth(row: &ProjectRow, text: &str, ctx: &ExecContext<'_>) -> TermTruth {
    let path = row
        .primary_location
        .as_ref()
        .map(|l| l.path_display.as_str());
    TermTruth::known(
        folded_contains(Some(row.name.as_str()), text)
            || folded_contains(row.owner.as_deref(), text)
            || folded_contains(row.description.as_deref(), text)
            || folded_contains(path, text)
            || folded_contains(row.last_commit_subject.as_deref(), text)
            || ctx
                .commit_subject_hits
                .is_some_and(|h| h.contains(&row.id.0)),
    )
}

fn text_truth(
    row: &LoadedRow,
    field: TextField,
    value: &str,
    quoted: bool,
    ctx: &ExecContext<'_>,
) -> TermTruth {
    let r = &row.row;
    match field {
        TextField::Lang => TermTruth::from_bool(
            r.primary_language
                .as_ref()
                .map(|l| l.eq_ignore_ascii_case(value)),
        ),
        TextField::Owner => {
            TermTruth::from_bool(r.owner.as_ref().map(|o| o.eq_ignore_ascii_case(value)))
        }
        TextField::Collection => match ctx.collection_ids_by_name.get(&value.to_lowercase()) {
            // An unknown collection name matches nothing; it is not unknown state.
            None => TermTruth::False,
            Some(id) => TermTruth::known(r.collection_ids.contains(&CollectionId(*id))),
        },
        TextField::In => {
            let wanted = value.to_lowercase();
            if wanted == "local" || wanted == "wsl" || wanted.starts_with("wsl:") {
                let Some(kind) = row.facts.location_kind else {
                    return TermTruth::Unknown;
                };
                return match wanted.as_str() {
                    "local" => TermTruth::known(kind != LocationKind::Wsl),
                    "wsl" => TermTruth::known(kind == LocationKind::Wsl),
                    other => TermTruth::known(
                        kind == LocationKind::Wsl
                            && row
                                .facts
                                .distro
                                .as_deref()
                                .unwrap_or("")
                                .eq_ignore_ascii_case(other.split_once(':').map_or("", |(_, d)| d)),
                    ),
                };
            }
            let Some(path) = r.primary_location.as_ref().map(|l| l.path_display.as_str()) else {
                return TermTruth::Unknown;
            };
            TermTruth::known(if quoted && ctx.paths_are_case_sensitive {
                path.starts_with(value)
            } else {
                path.to_lowercase().starts_with(&value.to_lowercase())
            })
        }
    }
}

fn flag_truth(row: &LoadedRow, flag: IsFlag, ctx: &ExecContext<'_>) -> TermTruth {
    let r = &row.row;
    match flag {
        IsFlag::Dirty => TermTruth::from_bool(r.is_dirty),
        IsFlag::Unpushed => TermTruth::from_count(r.ahead),
        IsFlag::Behind => TermTruth::from_count(r.behind),
        // No J1 result means no answer — an absent `interrupted_op` is not "not interrupted".
        IsFlag::Interrupted => {
            TermTruth::from_bool(r.refstate_observed_at.map(|_| r.interrupted_op.is_some()))
        }
        IsFlag::Archived => TermTruth::known(r.is_archived),
        IsFlag::Pinned => TermTruth::known(r.is_pinned),
        IsFlag::Hidden => TermTruth::known(r.is_hidden),
        IsFlag::Reference => TermTruth::known(r.is_reference),
        IsFlag::Bare => TermTruth::known(r.is_bare),
        IsFlag::Fork => TermTruth::known(r.is_fork),
        IsFlag::Empty => {
            TermTruth::from_bool(r.condition_signal.map(|s| s == ConditionSignal::Empty))
        }
        IsFlag::Shallow => TermTruth::known(r.is_shallow),
        IsFlag::Local => {
            TermTruth::from_bool(row.facts.location_kind.map(|k| k != LocationKind::Wsl))
        }
        IsFlag::Wsl => {
            TermTruth::from_bool(row.facts.location_kind.map(|k| k == LocationKind::Wsl))
        }
        IsFlag::New => match ctx.first_run_completed_at {
            None => TermTruth::Unknown,
            Some(stamp) => TermTruth::known(r.acknowledged_at.is_none() && r.created_at > stamp),
        },
    }
}

fn has_truth(row: &LoadedRow, attribute: HasAttribute) -> TermTruth {
    match attribute {
        HasAttribute::Stash => TermTruth::from_count(row.row.stash_count),
        HasAttribute::Remote => TermTruth::known(row.facts.has_remote),
        HasAttribute::Submodules => TermTruth::known(row.facts.has_submodules),
        HasAttribute::Readme => TermTruth::from_bool(row.facts.has_readme),
        // No column in phase 1 — `partition_answerable` removes these before we get here.
        HasAttribute::License | HasAttribute::Tests | HasAttribute::Ci => TermTruth::Unknown,
    }
}

#[must_use]
pub fn term_truth(row: &LoadedRow, term: &QueryTerm, ctx: &ExecContext<'_>) -> TermTruth {
    let r = &row.row;
    match term {
        QueryTerm::Bare { text, .. } => bare_truth(r, text, ctx),
        QueryTerm::Text {
            field,
            value,
            quoted,
            ..
        } => text_truth(row, *field, value, *quoted, ctx),
        QueryTerm::Flag { flag, .. } => flag_truth(row, *flag, ctx),
        QueryTerm::Has { attribute, .. } => has_truth(row, *attribute),
        QueryTerm::Size { op, bytes, .. } => match r.size_tracked_bytes {
            None => TermTruth::Unknown,
            Some(size) => {
                let size = u64::try_from(size).unwrap_or(0);
                TermTruth::known(match op {
                    Cmp::Gt => size > *bytes,
                    Cmp::Lt => size < *bytes,
                })
            }
        },
        // Seconds, not a float division: `age_days > days` and `age_secs > days * DAY` agree
        // exactly over integers, and the renderer's float form cuts at the same boundary.
        QueryTerm::TouchedAge { op, days, .. } => {
            let age = ctx.now - r.last_touched_at;
            let bound = i64::from(*days) * DAY;
            TermTruth::known(match op {
                Cmp::Gt => age > bound,
                Cmp::Lt => age < bound,
            })
        }
        QueryTerm::TouchedYear { year, .. } => TermTruth::known(
            i32::try_from(*year)
                .is_ok_and(|y| local_year(r.last_touched_at, ctx.tz_offset_min) == y),
        ),
    }
}

/// True when the term can be answered for *any* row, given what the index holds in phase 1.
fn answerable(term: &QueryTerm) -> bool {
    !matches!(
        term,
        QueryTerm::Has {
            attribute: HasAttribute::License | HasAttribute::Tests | HasAttribute::Ci,
            ..
        }
    )
}

fn render_term(term: &QueryTerm) -> String {
    match term {
        QueryTerm::Has { attribute, negated } => {
            let name = match attribute {
                HasAttribute::License => "license",
                HasAttribute::Tests => "tests",
                HasAttribute::Ci => "ci",
                HasAttribute::Readme => "readme",
                HasAttribute::Remote => "remote",
                HasAttribute::Stash => "stash",
                HasAttribute::Submodules => "submodules",
            };
            format!("{}has:{name}", if *negated { "-" } else { "" })
        }
        _ => String::new(),
    }
}

#[must_use]
pub fn partition_answerable(
    ast: &QueryAst,
    _ctx: &ExecContext<'_>,
) -> (Vec<QueryTerm>, Vec<IgnoredTerm>) {
    let mut runnable = Vec::new();
    let mut ignored = Vec::new();
    for term in &ast.terms {
        if answerable(term) {
            runnable.push(term.clone());
        } else {
            // `NotComputed` and not a fourth variant: nothing writes these columns in phase 1,
            // which is literally "not computed". `notAvailable` is the renderer's word for a
            // *projection* that cannot answer what the index can, and never crosses the wire.
            ignored.push(IgnoredTerm {
                text: render_term(term),
                reason: IgnoredReason::NotComputed,
            });
        }
    }
    (runnable, ignored)
}

/// §8.0b. A **negated** `is:hidden` is not a request for hidden rows.
#[must_use]
pub fn in_base_set(row: &ProjectRow, ast: &QueryAst) -> bool {
    let asks_for = |wanted: IsFlag| {
        ast.terms
            .iter()
            .any(|t| matches!(t, QueryTerm::Flag { flag, negated: false } if *flag == wanted))
    };
    if row.is_reference && !asks_for(IsFlag::Reference) {
        return false;
    }
    if row.is_hidden && !asks_for(IsFlag::Hidden) {
        return false;
    }
    true
}

#[derive(Debug)]
pub struct Executed<'r> {
    pub rows: Vec<&'r LoadedRow>,
    pub ignored: Vec<IgnoredTerm>,
}

#[must_use]
pub fn evaluate_query<'r>(
    rows: &'r [LoadedRow],
    ast: &QueryAst,
    ctx: &ExecContext<'_>,
) -> Executed<'r> {
    let (runnable, mut ignored) = partition_answerable(ast, ctx);
    let matched = rows
        .iter()
        .filter(|row| in_base_set(&row.row, ast))
        .filter(|row| {
            runnable
                .iter()
                .all(|term| match term_truth(row, term, ctx) {
                    // Unknown matches neither polarity. This line is the ruling.
                    TermTruth::Unknown => false,
                    TermTruth::True => !term.negated(),
                    TermTruth::False => term.negated(),
                })
        })
        .collect();
    let mut all = ast.ignored.clone();
    all.append(&mut ignored);
    Executed {
        rows: matched,
        ignored: all,
    }
}

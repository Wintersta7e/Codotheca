//! `codotheca-sidecar-fixture` — plant an index the core cannot open, for the end-to-end runs of
//! the startup report windows (§11.2a) and of the rebuild chain (§48.7.1), and read what the chain
//! left once the app has quit.
//!
//! Every line this binary prints goes to **stderr**: §2.1 gives stdout to protocol frames and
//! nothing else, and the crate denies `clippy::print_stdout`. What it planted is written to
//! `<data-dir>/fixture.json` and what it found to `<data-dir>/verify.json`, which is what a spec
//! reads.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use codotheca_core::index::sidecar::{
    counts, export, read, RestoreRule, Scope, SectionRow, Sidecar, SECTIONS,
};
use codotheca_core::index::{Index, IndexError};
use codotheca_core::ledger::price::record_ledger_fingerprint;
use codotheca_core::motivation::acknowledge::acknowledge;
use codotheca_core::motivation::keys::KEY_MOTIVATION_DAY_ONE;
use codotheca_core::protocol::MotivationAck;
use codotheca_core::testing::sidecar::{
    build_library, compare_documents, plant_startup_failure, FixtureLibrary, StartupPlant,
};
use codotheca_core::testing::zone::FixedZone;
use serde_json::{json, Value};

const USAGE: &str =
    "usage: codotheca-sidecar-fixture plant --kind corrupt|future|migration-failed \
                     --data-dir <dir>
       codotheca-sidecar-fixture plant --kind populated --data-dir <dir> --repos <dir>
       codotheca-sidecar-fixture verify --data-dir <dir>";

/// The result file, beside the planted index.
const FIXTURE_FILE: &str = "fixture.json";

/// What `verify` found, beside the rebuilt index.
const VERIFY_FILE: &str = "verify.json";

/// The populated plant's sidecar as it was exported, for `verify` to compare the rebuilt index
/// with: the app rewrites the sidecar itself, and the copy the rebuild sets aside is named by
/// the time it ran.
const PLANTED_SIDECAR: &str = "planted-sidecar.json";

/// A fixed clock: the plant is test data, and no window this feeds prints the time it was made.
const PLANTED_AT: i64 = 1_700_000_000;

/// After every repository's fixed commit date and every planted row.
const VERIFIED_AT: i64 = PLANTED_AT + 86_400;

enum Mode {
    Startup(StartupPlant, PathBuf),
    Populated { data: PathBuf, repos: PathBuf },
    Verify(PathBuf),
}

fn parse() -> Option<Mode> {
    let mut args = std::env::args().skip(1);
    let verb = args.next()?;
    let (mut kind, mut data, mut repos) = (None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--kind" => kind = args.next(),
            "--data-dir" => data = args.next().map(PathBuf::from),
            "--repos" => repos = args.next().map(PathBuf::from),
            _ => return None,
        }
    }
    match (verb.as_str(), kind.as_deref(), repos) {
        ("plant", Some("populated"), Some(repos)) => Some(Mode::Populated { data: data?, repos }),
        ("plant", Some(word), None) => Some(Mode::Startup(StartupPlant::from_word(word)?, data?)),
        ("verify", None, None) => Some(Mode::Verify(data?)),
        _ => None,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The projects the shelf lists once a scan finds them again: each visible project with a copy
/// still on disk, by the name its card shows.
fn shelf(index: &Index) -> Result<Vec<String>, IndexError> {
    let names = index
        .conn()
        .prepare(
            "SELECT DISTINCT p.name FROM project p JOIN location l ON l.project_id = p.id
              WHERE p.is_hidden = 0 AND l.presence = 'present' ORDER BY p.name",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(names)
}

/// A library, its sidecar, then the index corrupted beside a junk journal pair: the
/// `corrupt_index` window, with everything a rebuild restores waiting in the sidecar.
fn plant_populated(data: &Path, repos: &Path) -> Result<Value, IndexError> {
    let FixtureLibrary {
        mut index,
        subjects,
        ..
    } = build_library(data, repos, PLANTED_AT)?;
    // An installed user has seen the band before, so day one is a past date the sidecar carries;
    // without it the first paint after the rebuild stamps today, which no planted row says. It is
    // written the way the app writes it: the band's first paint, acknowledged at `PLANTED_AT`.
    index.with_tx(|tx| {
        acknowledge(
            tx,
            MotivationAck::BandShown,
            None,
            PLANTED_AT,
            &FixedZone(0),
        )?;
        Ok(())
    })?;
    // Every start records the ledger's fingerprint, so an index the app ran over always carries
    // it — and the rebuilt one will, before `verify` compares the two.
    record_ledger_fingerprint(index.conn())?;
    let written = index.export_sidecar(PLANTED_AT + 10)?;
    let shelf = shelf(&index)?;
    drop(index);
    std::fs::copy(&written.path, data.join(PLANTED_SIDECAR))?;
    let files = plant_startup_failure(StartupPlant::Corrupt, data, PLANTED_AT)?;

    let db = file_name(&Index::db_path(data));
    let quarantinable = [
        db.clone(),
        format!("{db}-wal"),
        format!("{db}-shm"),
        file_name(&written.path),
    ];
    let immediate: Vec<&str> = SECTIONS
        .iter()
        .filter(|s| matches!(s.scope, Scope::Global | Scope::NoScan))
        .map(|s| s.name)
        .collect();
    Ok(json!({
        "kind": "populated",
        "files": files,
        "quarantinable": quarantinable,
        "counts": written.counts,
        "subjects": subjects,
        "immediate": immediate,
        "shelf": shelf,
    }))
}

fn sessions_of(doc: &Sidecar, subject: &str) -> usize {
    doc.payload
        .projects
        .iter()
        .filter(|p| p.subject == subject)
        .map(|p| p.sessions.len())
        .sum()
}

/// `ledger_floor` is one row of no project; every field the planted row stores is a level the
/// restore may raise and never lower, so each must come back at least as high.
fn floor_lowered(planted: &[SectionRow], after: &[SectionRow]) -> Vec<String> {
    let ([was], [now]) = (planted, after) else {
        return vec![format!(
            "ledger_floor: {} rows planted, {} after the rebuild",
            planted.len(),
            after.len()
        )];
    };
    let Value::Object(fields) = &was.data else {
        return vec![format!("ledger_floor: a planted row of {}", was.data)];
    };
    fields
        .iter()
        .filter(|(field, level)| {
            !matches!(
                (level.as_u64(), now.data.get(field.as_str()).and_then(Value::as_u64)),
                (Some(before), Some(restored)) if restored >= before
            )
        })
        .map(|(field, level)| {
            let restored = now.data.get(field.as_str()).unwrap_or(&Value::Null);
            format!("ledger_floor: {field} planted {level}, {restored} after the rebuild")
        })
        .collect()
}

/// The raise-only section the app itself writes to at start: `rhythm_week`.
const RHYTHM_WEEK: &str = "rhythm_week";

/// `rhythm_week` rows by their week.
fn weeks(rows: &[SectionRow]) -> BTreeMap<String, &Value> {
    rows.iter()
        .map(|row| {
            let week = row.data.get("week_start").and_then(Value::as_str);
            (week.unwrap_or_default().to_owned(), &row.data)
        })
        .collect()
}

/// `rhythm_week` is raise-only, and once the rebuilt index has finished a scan, the next due pass
/// settles every week completed since the newest planted one (§39.7), on the real clock — the
/// spec's scan finishes before it closes the app, so whether a pass falls in between is timing.
/// So each planted week must come back at least as high — more active days, `missed`/`waived` to
/// `counted`, `partial` to `complete`, its target and settlement time unchanged — and any other
/// row must be a later week.
fn weeks_lowered(planted: &[SectionRow], after: &[SectionRow]) -> Vec<String> {
    let (was, now) = (weeks(planted), weeks(after));
    let newest = was.keys().next_back().cloned().unwrap_or_default();
    let mut out: Vec<String> = now
        .keys()
        .filter(|week| !was.contains_key(*week) && **week <= newest)
        .map(|week| format!("{RHYTHM_WEEK}: {week} appeared at or before the newest planted week"))
        .collect();
    for (week, before) in &was {
        let Some(restored) = now.get(week) else {
            out.push(format!(
                "{RHYTHM_WEEK}: {week} planted, absent after the rebuild"
            ));
            continue;
        };
        let field = |row: &Value, name: &str| row.get(name).cloned().unwrap_or(Value::Null);
        let raised = |name: &str, to: &str| {
            let (b, r) = (field(before, name), field(restored, name));
            b == r || r.as_str() == Some(to)
        };
        let days = |row: &Value| row.get("active_days").and_then(Value::as_u64);
        let held = days(restored) >= days(before)
            && raised("verdict", "counted")
            && raised("git_coverage", "complete")
            && field(before, "target") == field(restored, "target")
            && field(before, "settled_at") == field(restored, "settled_at");
        if !held {
            out.push(format!(
                "{RHYTHM_WEEK}: {week} planted {before}, {restored} after the rebuild"
            ));
        }
    }
    out
}

/// The run state the first tick's day close writes (§40.6 rule 3); it is not rebuild-owned.
const LIFELINE_RUN: &str = "badges.lifeline_run";

/// The latch sections, by their registered rule: the awards, which the app writes at start.
fn latches() -> impl Iterator<Item = &'static str> {
    SECTIONS
        .iter()
        .filter(|s| matches!(s.rule, RestoreRule::Latch))
        .map(|s| s.name)
}

/// A latch section's rows are keyed by `kind`, and the first tick's day close sweeps every
/// unlatched kind (§40.6), so the rebuilt index may hold an award the planted one did not —
/// earned from the restored history, such as a backdated Comeback. Each planted row must come
/// back exactly as it was, and any other row must be a kind the planted document did not hold.
fn latch_lowered(name: &str, planted: &[SectionRow], after: &[SectionRow]) -> Vec<String> {
    let kind = |row: &SectionRow| {
        row.data
            .get("kind")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let mut out: Vec<String> = planted
        .iter()
        .filter(|was| !after.contains(was))
        .map(|was| format!("{name}: planted {}, not restored unchanged", was.data))
        .collect();
    out.extend(
        after
            .iter()
            .filter(|now| {
                !planted.contains(now) && planted.iter().any(|was| kind(was) == kind(now))
            })
            .map(|now| format!("{name}: a planted kind came back as {}", now.data)),
    );
    out
}

/// `doc` without what the app writes at start — `rhythm_week` and the latch sections, which
/// [`weeks_lowered`] and [`latch_lowered`] compare instead, and the Lifeline run state.
fn without_start_writes(doc: &Sidecar) -> Sidecar {
    let mut doc = doc.clone();
    doc.payload.sections.remove(RHYTHM_WEEK);
    for name in latches() {
        doc.payload.sections.remove(name);
    }
    doc.payload.settings.remove(LIFELINE_RUN);
    doc
}

/// The index the app left, exported and compared with the planted document.
fn verify(data: &Path) -> Result<Value, IndexError> {
    let planted = read(&data.join(PLANTED_SIDECAR))?;
    let index = Index::open_at(data, VERIFIED_AT)?;
    let after = export(index.conn(), planted.generation, VERIFIED_AT)?;
    let comparison = compare_documents(
        &without_start_writes(&planted),
        &without_start_writes(&after),
    )?;
    let pending: i64 = index
        .conn()
        .query_row("SELECT COUNT(*) FROM sidecar_pending", [], |r| r.get(0))?;
    // Read from the index, not the export: a key that stopped travelling would leave both
    // documents without it, and the comparison above could not see the user's day one reset.
    let day_one: Vec<String> = index
        .conn()
        .prepare("SELECT v FROM app_meta WHERE k = ?1")?
        .query_map([KEY_MOTIVATION_DAY_ONE], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let was: Vec<String> = planted
        .payload
        .settings
        .get(KEY_MOTIVATION_DAY_ONE)
        .cloned()
        .into_iter()
        .collect();
    let mut differences = comparison.differences;
    if was.is_empty() || day_one != was {
        differences.push(format!(
            "settings: {KEY_MOTIVATION_DAY_ONE} planted {was:?}, {day_one:?} after the rebuild"
        ));
    }

    let now = counts(&after);
    let counted: BTreeMap<String, [u64; 2]> = counts(&planted)
        .into_iter()
        .map(|(key, n)| {
            let restored = now.get(&key).copied().unwrap_or(0);
            (key, [n, restored])
        })
        .collect();
    let session_mismatch: Vec<&str> = planted
        .payload
        .projects
        .iter()
        .filter(|p| sessions_of(&after, &p.subject) != p.sessions.len())
        .map(|p| p.subject.as_str())
        .collect();
    let raise_only: Vec<&str> = SECTIONS
        .iter()
        .filter(|s| matches!(s.rule, RestoreRule::RaiseOnly))
        .map(|s| s.name)
        .collect();
    let rows = |doc: &Sidecar, name: &str| -> Vec<SectionRow> {
        doc.payload.sections.get(name).cloned().unwrap_or_default()
    };
    // Each raise-only section brings the comparison its values need; one without it is named
    // here rather than passed unchecked.
    let lowered: Vec<String> = raise_only
        .iter()
        .flat_map(|&name| match name {
            "ledger_floor" => floor_lowered(&rows(&planted, name), &rows(&after, name)),
            RHYTHM_WEEK => weeks_lowered(&rows(&planted, name), &rows(&after, name)),
            _ => vec![format!(
                "{name}: no raise-only comparison is written for this section"
            )],
        })
        .chain(
            latches()
                .flat_map(|name| latch_lowered(name, &rows(&planted, name), &rows(&after, name))),
        )
        .collect();
    Ok(json!({
        "counts": counted,
        "compared": comparison.compared.values().sum::<usize>()
            + rows(&planted, RHYTHM_WEEK).len()
            + latches().map(|name| rows(&planted, name).len()).sum::<usize>(),
        "differences": differences,
        "pending": pending,
        "sessionMismatch": session_mismatch,
        "raiseOnly": raise_only.len(),
        "lowered": lowered,
    }))
}

fn main() -> ExitCode {
    let mut err = std::io::stderr();
    let Some(mode) = parse() else {
        let _ = writeln!(err, "{USAGE}");
        return ExitCode::FAILURE;
    };
    let (dir, file, outcome) = match &mode {
        Mode::Startup(plant, dir) => (
            dir,
            FIXTURE_FILE,
            plant_startup_failure(*plant, dir, PLANTED_AT)
                .map(|files| json!({ "kind": plant.word(), "files": files })),
        ),
        Mode::Populated { data, repos } => (data, FIXTURE_FILE, plant_populated(data, repos)),
        Mode::Verify(data) => (data, VERIFY_FILE, verify(data)),
    };
    let found = match outcome {
        Ok(found) => found,
        Err(e) => {
            let _ = writeln!(err, "writing {file} failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(dir.join(file), found.to_string()) {
        let _ = writeln!(err, "writing {file} failed: {e}");
        return ExitCode::FAILURE;
    }
    let _ = writeln!(err, "{file}: {found}");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::{latch_lowered, latches, SectionRow};
    use serde_json::json;

    fn award(kind: &str, landed_on: Option<&str>) -> SectionRow {
        SectionRow {
            subject: None,
            location_keys: Vec::new(),
            data: json!({ "kind": kind, "earned_on": "2023-10-15", "landed_on": landed_on }),
        }
    }

    #[test]
    fn the_registry_holds_a_latch_section_to_compare() {
        assert!(latches().count() > 0, "no latch section is registered");
    }

    #[test]
    fn an_award_the_first_sweep_earns_is_not_a_difference() {
        let planted = [
            award("curator", None),
            award("marathon", Some("2026-01-02")),
        ];
        let after = [
            award("marathon", Some("2026-01-02")),
            award("comeback", None),
            award("curator", None),
        ];
        assert_eq!(
            latch_lowered("awards", &planted, &after),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_planted_award_that_changed_or_vanished_is_named() {
        let planted = [
            award("curator", None),
            award("marathon", Some("2026-01-02")),
        ];
        let after = [award("marathon", None)];
        let found = latch_lowered("awards", &planted, &after);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(
            found.iter().any(|line| line.contains("\"curator\"")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|line| line.contains("came back as")),
            "{found:?}"
        );
    }
}

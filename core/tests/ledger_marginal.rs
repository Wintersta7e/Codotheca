//! §38.11's marginal price against the rows its payout path then writes, and §38.8.4's verdict
//! writers.
//!
//! The closure is real: J7 closing a TODO marker inside its own transaction, then the settle's
//! completion evaluator lighting `remote`, which passes once `project.remote_key` is set under an
//! observed refstate. The quote is taken before it and compared with what the XP moved by.

#![cfg(feature = "testkit")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use codotheca_core::cancel::CancelToken;
use codotheca_core::clock::LocalInstant;
use codotheca_core::completion::evaluate::CheckRow;
use codotheca_core::debt::identity::DebtKey;
use codotheca_core::debt::store::{DebtCloseReason, DebtClosure, SweepEffect};
use codotheca_core::debt::xp::pay_debt_day;
use codotheca_core::git::{JobClass, JobContext, RepoHandle, TreeEntry};
use codotheca_core::health::switches::write_switches;
use codotheca_core::index::subject::subject_for_project;
use codotheca_core::index::{Index, IndexError};
use codotheca_core::jobs::j7_markers;
use codotheca_core::ledger::latch::latch_completion_checks;
use codotheca_core::ledger::marginal::{marginal_xp, Closure};
use codotheca_core::ledger::price::PRICES;
use codotheca_core::ledger::verdict::{pay_prepass_award, pay_verdict_award, VerdictAward};
use codotheca_core::ledger::xp::xp_by_term;
use codotheca_core::mount::StoreClass;
use codotheca_core::protocol::{
    CheckState, CompletionCheck, DebtScoring, DebtSource, HealthCheckSwitch, HealthDetectedIn,
    LocationId, ProjectId,
};
use codotheca_core::testing::{FakeGitBackend, GitReply};
use serde_json::Value;

/// The clock every write here is dated at.
const NOW: i64 = 1_800_000_000;
/// Enrolment happened before any evidence below.
const ACK: i64 = NOW - 1_000;
/// The instant and offset every payout and every quote is framed in.
const AT: LocalInstant = LocalInstant {
    unix: NOW,
    offset_min: 0,
};

/// A closure that pays a `debt_day` for a scored item and lights `remote`.
const BOTH: Closure = Closure {
    debt_day: Some(DebtScoring::Scored),
    lights: Some(CompletionCheck::Remote),
    verdict: false,
};

/// One project on one present location with an observed refstate.
struct Project {
    id: ProjectId,
    location: LocationId,
}

struct Rig {
    _dir: tempfile::TempDir,
    index: Mutex<Index>,
    git: Arc<FakeGitBackend>,
    repo: RepoHandle,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open_at(dir.path(), NOW).unwrap();
        index
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO app_meta (k, v) VALUES ('content_scan_enabled', '1')
                     ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                    [],
                )?;
                write_switches(
                    tx,
                    &[
                        HealthCheckSwitch {
                            check: DebtSource::MissingTests,
                            enabled: false,
                        },
                        HealthCheckSwitch {
                            check: DebtSource::UnpushedCommits,
                            enabled: false,
                        },
                    ],
                )
            })
            .unwrap();
        Self {
            _dir: dir,
            index: Mutex::new(index),
            git: Arc::new(FakeGitBackend::new()),
            repo: RepoHandle::bare(
                std::path::Path::new("/does/not/matter"),
                codotheca_core::git::StoreKey::new("test-store"),
                StoreClass::Local,
            ),
        }
    }

    fn tx<T>(&self, f: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, IndexError>) -> T {
        self.index.lock().unwrap().with_tx(f).unwrap()
    }

    /// Not Reference, three `commit_day` rows (the completion gate's floor), `remote` failing.
    fn project(&self, name: &str, authored: Option<i64>, enrolled: bool) -> Project {
        let (id, location) = self.tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, lineage_key, authored_by_user,
                                      is_reference, acknowledged_at, created_at, updated_at)
                 VALUES (?1, ?1, ?1, ?2, 0, ?3, 0, 0)",
                rusqlite::params![name, authored, enrolled.then_some(ACK)],
            )?;
            let id = ProjectId(tx.last_insert_rowid());
            let path = format!("/{name}");
            tx.execute(
                "INSERT INTO location
                   (project_id, kind, path_bytes, path_key, path_display, store_key, presence,
                    repo_kind, head_oid, refstate_observed_at)
                 VALUES (?1, 'linux', ?2, ?2, ?3, 'store-a', 'present', 'worktree', 'head-0', ?4)",
                rusqlite::params![id.0, path.as_bytes(), path, NOW],
            )?;
            let location = LocationId(tx.last_insert_rowid());
            for day in 1..=3_i64 {
                tx.execute(
                    "INSERT INTO xp_events (ts, tz_offset_min, project_id, subject_key, kind,
                                            dedupe_key, track)
                     VALUES (?1, 0, ?2, ?3, 'commit_day', ?4, 'git')",
                    rusqlite::params![
                        NOW - 86_400 * day,
                        id.0,
                        format!("lineage:{name}"),
                        format!("cd:{name}:{day}")
                    ],
                )?;
            }
            Ok((id, location))
        });
        Project { id, location }
    }

    /// A project with no lineage and no location: no subject any row could be keyed on.
    fn unkeyed(&self, name: &str) -> ProjectId {
        self.tx(|tx| {
            tx.execute(
                "INSERT INTO project (name, seed_basename, authored_by_user, is_reference,
                                      acknowledged_at, created_at, updated_at)
                 VALUES (?1, ?1, 1, 0, ?2, 0, 0)",
                rusqlite::params![name, ACK],
            )?;
            Ok(ProjectId(tx.last_insert_rowid()))
        })
    }

    fn remote(&self, p: &Project) {
        self.tx(|tx| {
            tx.execute(
                "UPDATE project SET remote_key = 'remote-key' WHERE id = ?1",
                [p.id.0],
            )?;
            Ok(())
        });
    }

    /// One real closure: J7 over a HEAD carrying `markers` TODO lines, then the settle's
    /// completion evaluator, each in its own transaction.
    fn close(&self, p: &Project, head: &str, markers: usize) {
        let mut bytes = String::new();
        for i in 0..markers {
            let _ = writeln!(bytes, "// TODO: marker number {i}");
        }
        let oid = format!("{head:a>40}");
        self.git.script_blob(&oid, bytes.as_bytes());
        self.git.always_head_tree(GitReply::Ok(vec![TreeEntry {
            mode: "100644".to_owned(),
            kind: "blob".to_owned(),
            oid,
            path: b"src/a.rs".to_vec(),
        }]));
        self.tx(|tx| {
            tx.execute(
                "UPDATE location SET head_oid = ?2 WHERE id = ?1",
                rusqlite::params![p.location.0, head],
            )?;
            Ok(())
        });
        let cancel = CancelToken::new();
        let ctx = JobContext::new(JobClass::Background, &cancel, None);
        j7_markers::run_j7(
            &self.index,
            self.git.as_ref(),
            &self.repo,
            &ctx,
            j7_markers::ScanRun {
                project: p.id,
                location: p.location,
                cursor: None,
                at: AT,
                detected_in: HealthDetectedIn::Foreground,
                announce: None,
            },
        )
        .unwrap();
        self.tx(|tx| codotheca_core::completion::evaluate_and_write(tx, p.id, AT).map(drop));
    }

    fn quote(&self, project: ProjectId, closure: Closure) -> Option<u32> {
        let guard = self.index.lock().unwrap();
        let quote = marginal_xp(guard.conn(), project, &closure, AT).unwrap();
        drop(guard);
        quote
    }

    fn xp(&self) -> i64 {
        let guard = self.index.lock().unwrap();
        let xp = xp_by_term(guard.conn()).unwrap().total();
        drop(guard);
        xp
    }

    /// The quote before one closure, and what the XP moved by across it.
    fn quote_then_close(&self, p: &Project, head: &str, markers: usize) -> (Option<u32>, i64) {
        let quote = self.quote(p.id, BOTH);
        let before = self.xp();
        self.close(p, head, markers);
        (quote, self.xp() - before)
    }

    fn rows(&self, sql: &str, project: Option<ProjectId>) -> Vec<Value> {
        let guard = self.index.lock().unwrap();
        let mut stmt = guard.conn().prepare(sql).unwrap();
        let rows = stmt
            .query_map([project.map(|p| p.0)], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|meta| serde_json::from_str(&meta.unwrap()).unwrap())
            .collect();
        drop(stmt);
        drop(guard);
        rows
    }

    fn verdicts(&self, project: ProjectId) -> Vec<Value> {
        self.rows(
            "SELECT meta FROM xp_events WHERE kind = 'amnesty_verdict' AND project_id = ?1
              ORDER BY id",
            Some(project),
        )
    }

    fn prepasses(&self) -> Vec<Value> {
        self.rows(
            "SELECT meta FROM xp_events WHERE kind = 'amnesty_verdict' AND project_id IS ?1
              ORDER BY id",
            None,
        )
    }
}

/// One scored item fixed: the sweep effect a paying `debt_day` closure hands its writer.
fn one_fixed() -> SweepEffect {
    SweepEffect {
        closed: vec![DebtClosure {
            key: DebtKey::content("subject", &"a".repeat(64), 0),
            reason: DebtCloseReason::Fixed,
            scoring: DebtScoring::Scored,
        }],
        ..SweepEffect::default()
    }
}

/// The quote taken before a closure is what the closure then pays.
fn assert_paid_as_quoted(what: &str, (quote, paid): (Option<u32>, i64), expected: i64) {
    eprintln!("{what}: quoted {quote:?}, paid {paid}");
    assert_eq!(quote.map(i64::from), Some(paid), "{what}: quote and payout");
    assert_eq!(paid, expected, "{what}: priced from the table");
}

#[test]
fn ac_p4_38_18_quote_equals_payout() {
    let rig = Rig::new();
    let p = rig.project("mine", Some(1), true);
    rig.close(&p, "one", 5);
    rig.remote(&p);

    assert_paid_as_quoted(
        "a closure that pays the day and lights remote",
        rig.quote_then_close(&p, "two", 4),
        PRICES.debt_day + PRICES.completion_check,
    );
    assert_paid_as_quoted(
        "a second closure the same day",
        rig.quote_then_close(&p, "three", 3),
        0,
    );
    let latched = Closure {
        debt_day: None,
        lights: Some(CompletionCheck::Remote),
        verdict: false,
    };
    assert_eq!(rig.quote(p.id, latched), Some(0), "a latched check");

    // Not enrolled: the day is refused by the project's gate and the check still latches.
    let q = rig.project("unenrolled", Some(1), false);
    rig.close(&q, "u-one", 5);
    rig.remote(&q);
    assert_paid_as_quoted(
        "an unenrolled project",
        rig.quote_then_close(&q, "u-two", 4),
        PRICES.completion_check,
    );

    let orphan = rig.unkeyed("orphan");
    assert_eq!(
        rig.quote(orphan, BOTH),
        None,
        "no subject is not computable"
    );
    let unclassified = rig.project("unclassified", None, true);
    assert_eq!(
        rig.quote(unclassified.id, BOTH),
        None,
        "authorship not yet classified is not computable"
    );
}

#[test]
fn verdict_writers() {
    let rig = Rig::new();
    let card = rig.project("card", Some(1), true);
    let member = rig.project("member", Some(1), true);
    let verdict = Closure {
        debt_day: None,
        lights: None,
        verdict: true,
    };
    let before = rig.xp();

    let price = u32::try_from(PRICES.amnesty_verdict).unwrap();
    assert_eq!(rig.quote(card.id, verdict), Some(price));
    let first = rig.tx(|tx| pay_verdict_award(tx, card.id, "keep", AT));
    assert!(matches!(first, VerdictAward::Paid(_)), "{first:?}");
    let redealt = rig.tx(|tx| pay_verdict_award(tx, card.id, "reference", AT));
    assert_eq!(redealt, VerdictAward::AlreadyPaid);
    assert_eq!(rig.quote(card.id, verdict), Some(0));
    let rows = rig.verdicts(card.id);
    assert_eq!(rows.len(), 1, "card then re-dealt card: {rows:?}");
    assert_eq!(rows[0]["what"], "verdict");
    assert_eq!(rows[0]["verdict"], "keep");
    assert_eq!(rows[0]["when"], AT.date());
    assert_eq!(rows[0]["project_name"], "card");

    let admitted_at = NOW - 600;
    assert!(rig.tx(|tx| pay_prepass_award(tx, admitted_at, "abandoned", 2, AT)));
    assert!(!rig.tx(|tx| pay_prepass_award(tx, admitted_at, "abandoned", 2, AT)));
    let group = rig.prepasses();
    assert_eq!(group.len(), 1, "one approval, replayed: {group:?}");
    assert_eq!(group[0]["what"], "prepass");
    assert_eq!(group[0]["group"], "abandoned");
    assert_eq!(group[0]["projects"], 2);
    assert_eq!(group[0]["when"], AT.date());

    // A member of the approved group keeps its own once-ever award.
    assert_eq!(rig.quote(member.id, verdict), Some(price));
    let own = rig.tx(|tx| pay_verdict_award(tx, member.id, "archive", AT));
    assert!(matches!(own, VerdictAward::Paid(_)), "{own:?}");
    assert_eq!(rig.verdicts(member.id).len(), 1);

    let orphan = rig.unkeyed("orphan");
    let none = rig.tx(|tx| pay_verdict_award(tx, orphan, "keep", AT));
    assert_eq!(none, VerdictAward::NoSubject);
    assert_eq!(rig.verdicts(orphan), Vec::<Value>::new());

    assert_eq!(rig.xp() - before, 3 * PRICES.amnesty_verdict);
}

/// §38.11's `None` is per row: on a project whose authorship is not yet classified, only the
/// `completion_check` row reads the gate that cannot be evaluated (§38.8.2). A `debt_day` there
/// is quoted as its writer pays it, never as an unknown it is not. The writer is driven directly:
/// J7 does not run on a project J1.5 has not classified, so no marker closure reaches it.
#[test]
fn unclassified_authorship_leaves_only_the_check_unknown() {
    let rig = Rig::new();
    let p = rig.project("unclassified", None, true);
    let day = Closure {
        debt_day: Some(DebtScoring::Scored),
        lights: None,
        verdict: false,
    };
    let quote = rig.quote(p.id, day);
    let before = rig.xp();
    let paid = rig
        .tx(|tx| Ok(pay_debt_day(tx, p.id, &one_fixed(), AT)))
        .unwrap();
    assert!(paid.wrote_row, "{paid:?}");
    assert_paid_as_quoted(
        "a day-only closure on an unclassified project",
        (quote, rig.xp() - before),
        PRICES.debt_day,
    );

    let lights = Closure {
        debt_day: None,
        lights: Some(CompletionCheck::Remote),
        verdict: false,
    };
    assert_eq!(
        rig.quote(p.id, lights),
        None,
        "a closure lighting a check reads a gate that cannot be evaluated"
    );
}

/// A project awaiting restore of its own record is held before anything else is read
/// (§48.8.4): every writer writes nothing for it, so it quotes `0`. A project with no subject has
/// no record of its own, so nothing holds it and it quotes `None`.
#[test]
fn a_held_project_quotes_what_its_writers_write() {
    let rig = Rig::new();
    let held = rig.project("held", Some(1), true).id;
    let unkeyed = rig.unkeyed("unkeyed");
    rig.tx(|tx| {
        let key = subject_for_project(tx, held)?.expect("subject").to_key();
        tx.execute(
            "INSERT INTO sidecar_pending (source_generation, subject_key, location_keys, record,
                                         queued_at)
             VALUES (1, ?1, '[]', '{}', 1)",
            [key],
        )?;
        Ok(())
    });
    let every_row = Closure {
        debt_day: Some(DebtScoring::Scored),
        lights: Some(CompletionCheck::Remote),
        verdict: true,
    };
    assert_eq!(rig.quote(held, every_row), Some(0), "a held project");
    assert_eq!(rig.quote(unkeyed, every_row), None, "no subject, no hold");

    let before = rig.xp();
    let day = rig
        .tx(|tx| Ok(pay_debt_day(tx, held, &one_fixed(), AT)))
        .unwrap();
    assert!(!day.wrote_row, "{day:?}");
    let passing = [CheckRow {
        key: CompletionCheck::Remote,
        state: CheckState::Pass,
        user_na: None,
        unknown_reason: None,
        observed_at: NOW,
    }];
    assert_eq!(
        rig.tx(|tx| latch_completion_checks(tx, held, &passing, AT)),
        0
    );
    assert_eq!(
        rig.tx(|tx| pay_verdict_award(tx, held, "keep", AT)),
        VerdictAward::AwaitingRestore
    );
    assert_eq!(
        rig.rows(
            "SELECT json_object('kind', kind) FROM xp_events
              WHERE project_id = ?1 AND kind <> 'commit_day'",
            Some(held)
        ),
        Vec::<Value>::new()
    );
    assert_eq!(rig.xp() - before, 0);
}

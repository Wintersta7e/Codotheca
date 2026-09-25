#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! **AC-P2-21-6** — the observation clock, and its six divergent outcomes.
//!
//! The cases are asserted **against one fake in one test**, because it is their **divergence**
//! that is the contract: a 200 writes the values and dates them, a 304 dates them *without*
//! rewriting one, and a throttled, 401'd, 403'd, 404'd, rejected or skipped call dates **nothing
//! at all**. Asserted apart, each of those passes against an implementation that gets the others
//! wrong.
//!
//! The writers under test are **p2-25's own** (`core/src/remote/store.rs`), which is the point:
//! this exercises the writers the product actually uses, so a divergence between the two plans
//! shows up here rather than passing against a copy of them.

use std::sync::{Arc, Mutex};

use codotheca_core::accounts::keychain::{token_ref, SecretToken, TokenStore};
use codotheca_core::accounts::store::{insert_account, NewAccount};
use codotheca_core::http::{HttpResponse, HttpTransport};
use codotheca_core::index::Index;
use codotheca_core::protocol::{AuthKind, ProjectId, ScopeTier};
use codotheca_core::provider::GitHubProvider;
use codotheca_core::sync::http::ObservingTransport;
use codotheca_core::sync::outcome::SyncOutcome;
use codotheca_core::sync::tasks::remote::run_project_remote;
use codotheca_core::sync::SyncDeps;
use codotheca_core::testing::{FakeClock, FakeTokenStore, FakeTransport, TempIndex};

const NOW: i64 = 1_800_000_000;
const LATER: i64 = NOW + 7_200;
const HOST: &str = "forge.example.invalid";

struct Fixture {
    index: Arc<Mutex<Index>>,
    transport: Arc<FakeTransport>,
    clock: Arc<FakeClock>,
    deps: SyncDeps,
    project: ProjectId,
    _dir: tempfile::TempDir,
}

/// The row as it stands **before** the call under test: every value written and dated at `NOW`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    visibility: Option<String>,
    description: Option<String>,
    stars: Option<i64>,
    permitted: i64,
    observed_at: Option<i64>,
    etag: Option<String>,
    ci_observed_at: Option<i64>,
    topics: Vec<String>,
}

fn read_row(index: &Mutex<Index>) -> Row {
    let guard = index.lock().expect("index");
    let conn = guard.conn();
    let row = conn
        .query_row(
            "SELECT visibility, description, stars, permitted, observed_at, etag, ci_observed_at
               FROM remote_repo WHERE provider = 'github' AND provider_repo_id = '7'",
            [],
            |r| {
                Ok(Row {
                    visibility: r.get(0)?,
                    description: r.get(1)?,
                    stars: r.get(2)?,
                    permitted: r.get(3)?,
                    observed_at: r.get(4)?,
                    etag: r.get(5)?,
                    ci_observed_at: r.get(6)?,
                    topics: Vec::new(),
                })
            },
        )
        .expect("the facts row exists");
    let topics: Vec<String> = conn
        .prepare(
            "SELECT topic FROM remote_topic
              WHERE provider = 'github' AND provider_repo_id = '7' ORDER BY topic",
        )
        .expect("prepared")
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .map(Result::unwrap)
        .collect();
    Row { topics, ..row }
}

fn fixture() -> Fixture {
    let temp = TempIndex::new();
    let dir = tempfile::tempdir().expect("tmp");
    let mut index = Index::open_at(dir.path(), NOW).expect("index");
    drop(temp);

    let project = index
        .with_tx(|tx| {
            insert_account(
                tx,
                &NewAccount {
                    provider: "github".to_owned(),
                    host: HOST.to_owned(),
                    login: "owner".to_owned(),
                    display_name: None,
                    auth_kind: AuthKind::Device,
                    scope_tier: ScopeTier::Private,
                    granted_scopes: vec!["repo".to_owned()],
                    token_ref: token_ref("github", HOST, "owner"),
                },
                NOW,
            )
            .expect("account");
            tx.execute(
                "INSERT INTO project (name, seed_basename, remote_key, provider,
                                      provider_repo_id, remote_link_basis, created_at, updated_at)
                 VALUES ('alpha', 'alpha', ?1, 'github', '7', 'provider_id', ?2, ?2)",
                rusqlite::params![format!("{HOST}/owner/alpha"), NOW],
            )?;
            Ok(ProjectId(tx.last_insert_rowid()))
        })
        .expect("seed");

    let transport = Arc::new(FakeTransport::new());
    let clock = Arc::new(FakeClock::new(NOW));
    let observing = Arc::new(ObservingTransport::new(
        Arc::clone(&transport) as Arc<dyn HttpTransport>,
        Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
    ));
    let tokens = Arc::new(FakeTokenStore::available());
    tokens
        .store(
            &token_ref("github", HOST, "owner"),
            &SecretToken::new("t".to_owned()),
        )
        .expect("token");
    let provider = Arc::new(GitHubProvider::new(
        Arc::clone(&observing) as Arc<dyn HttpTransport>,
        HOST.to_owned(),
    ));

    Fixture {
        index: Arc::new(Mutex::new(index)),
        transport,
        clock: Arc::clone(&clock),
        deps: SyncDeps {
            provider,
            transport: observing,
            tokens,
            // **The same clock the decorator holds.** Two would let the observation and the write
            // disagree about when they happened, which is the one thing this file is about.
            clock: Arc::clone(&clock) as Arc<dyn codotheca_core::clock::Clock>,
            cancel: codotheca_core::cancel::CancelToken::new(),
            // UTC in a test, so a local date never depends on the machine running it.
            tz_offset_min: 0,
        },
        project,
        _dir: dir,
    }
}

/// The repository object, as `GET /repos/{owner}/{name}` renders it.
fn repo_body(description: &str, stars: u32) -> Vec<u8> {
    format!(
        r#"{{"id":7,"clone_url":"https://{HOST}/owner/alpha.git",
            "owner":{{"login":"owner","type":"User"}},"name":"alpha",
            "fork":false,"archived":false,"private":false,
            "description":"{description}","stargazers_count":{stars},"topics":["rust"],
            "permissions":{{"push":true}}}}"#
    )
    .into_bytes()
}

fn response(status: u16, headers: &[(&str, &str)], body: Vec<u8>) -> HttpResponse {
    HttpResponse {
        status,
        headers: codotheca_core::http::normalise_headers(headers.iter().copied()),
        body,
    }
}

/// The empty Actions answer, so the CI read that follows a good facts read does not consume a
/// scripted response meant for something else.
fn no_runs() -> HttpResponse {
    response(200, &[], br#"{"workflow_runs":[]}"#.to_vec())
}

/// Put the row in a known state — every value written, dated `NOW` — and return it.
fn seeded(f: &Fixture) -> Row {
    f.transport.push(response(
        200,
        &[("etag", "W/\"one\"")],
        repo_body("first", 3),
    ));
    f.transport.push(no_runs());
    let outcome = run_project_remote(&f.deps, &f.index, f.project).expect("seed read");
    assert_eq!(outcome, SyncOutcome::Done);
    let row = read_row(&f.index);
    assert_eq!(row.observed_at, Some(NOW), "the seed dated the row");
    row
}

/// **AC-P2-21-6, all of it, in one test.**
///
/// The length is the contract, which is why the lint is allowed rather than the test split:
/// §21.14's gate 6 requires these cases *"asserted against one fake in one test, because it is
/// their **divergence** that is the contract"*. Split into seven, each one passes against an
/// implementation that gets the other six wrong.
#[allow(clippy::too_many_lines)]
#[test]
fn a_200_writes_a_304_confirms_and_every_refusal_dates_nothing() {
    let mut exercised: Vec<&str> = Vec::new();

    // ---- 200: writes the values **and** moves the clock. -------------------------------------
    {
        let f = fixture();
        let before = seeded(&f);
        f.clock.set_unix(LATER);
        f.transport.push(response(
            200,
            &[("etag", "W/\"two\"")],
            repo_body("second", 9),
        ));
        f.transport.push(no_runs());
        assert_eq!(
            run_project_remote(&f.deps, &f.index, f.project).expect("ran"),
            SyncOutcome::Done
        );
        let after = read_row(&f.index);
        assert_eq!(
            after.description.as_deref(),
            Some("second"),
            "values rewritten"
        );
        assert_eq!(after.stars, Some(9));
        assert_eq!(after.etag.as_deref(), Some("W/\"two\""));
        assert_eq!(after.observed_at, Some(LATER), "and the clock moved");
        assert_ne!(after.description, before.description);
        exercised.push("200");
    }

    // ---- 304: moves the clock **without rewriting a value**, field by field. -----------------
    {
        let f = fixture();
        let before = seeded(&f);
        f.clock.set_unix(LATER);
        f.transport.push(response(304, &[], Vec::new()));
        f.transport.push(response(304, &[], Vec::new()));
        assert_eq!(
            run_project_remote(&f.deps, &f.index, f.project).expect("ran"),
            SyncOutcome::NotModified
        );
        let after = read_row(&f.index);
        assert_eq!(after.visibility, before.visibility);
        assert_eq!(after.description, before.description);
        assert_eq!(after.stars, before.stars);
        assert_eq!(after.topics, before.topics);
        assert_eq!(
            after.etag, before.etag,
            "the validator we sent is still ours"
        );
        assert_eq!(after.permitted, before.permitted);
        assert_eq!(
            after.observed_at,
            Some(LATER),
            "a 304 is an observation and not the absence of one"
        );
        exercised.push("304");
    }

    // ---- Every refusal: the clock stays **exactly** where it was, to the second. --------------
    for (name, first) in [
        (
            "throttled",
            response(
                429,
                &[("x-ratelimit-reset", "1800009999"), ("retry-after", "30")],
                Vec::new(),
            ),
        ),
        ("401", response(401, &[], Vec::new())),
        ("403", response(403, &[], Vec::new())),
        ("404", response(404, &[], Vec::new())),
        ("rejected", response(422, &[], Vec::new())),
    ] {
        let f = fixture();
        let before = seeded(&f);
        f.clock.set_unix(LATER);
        f.transport.push(first);
        let outcome = run_project_remote(&f.deps, &f.index, f.project).expect("ran");
        assert!(
            !matches!(outcome, SyncOutcome::Done | SyncOutcome::NotModified),
            "{name} settled as a success: {outcome:?}"
        );
        let after = read_row(&f.index);
        assert_eq!(
            after.observed_at, before.observed_at,
            "{name} moved the observation clock"
        );
        assert_eq!(
            after.ci_observed_at, before.ci_observed_at,
            "{name} moved the CI clock"
        );
        assert_eq!(
            after.description, before.description,
            "{name} rewrote a value"
        );
        assert_eq!(after.stars, before.stars, "{name} rewrote a value");
        exercised.push(name);
    }

    // ---- Skipped: no binding, so no request is issued and nothing is dated. -------------------
    {
        let f = fixture();
        let before = seeded(&f);
        let orphan = {
            let mut guard = f.index.lock().expect("index");
            guard
                .with_tx(|tx| {
                    tx.execute(
                        "INSERT INTO project (name, seed_basename, created_at, updated_at)
                         VALUES ('unbound', 'unbound', ?1, ?1)",
                        [NOW],
                    )?;
                    Ok(ProjectId(tx.last_insert_rowid()))
                })
                .expect("orphan")
        };
        let sent = f.transport.request_count();
        f.clock.set_unix(LATER);
        let outcome = run_project_remote(&f.deps, &f.index, orphan).expect("ran");
        assert_eq!(outcome, SyncOutcome::NotFound);
        assert_eq!(
            f.transport.request_count(),
            sent,
            "a project with no binding must not spend a request"
        );
        assert_eq!(read_row(&f.index).observed_at, before.observed_at);
        exercised.push("skipped");
    }

    // ---- `NextPage` is unreachable here, and that is asserted rather than assumed. ------------
    {
        let f = fixture();
        f.transport.push(response(
            200,
            &[("etag", "W/\"one\"")],
            repo_body("first", 3),
        ));
        f.transport.push(no_runs());
        let outcome = run_project_remote(&f.deps, &f.index, f.project).expect("ran");
        assert!(
            !matches!(outcome, SyncOutcome::NextPage { .. }),
            "a single-repository read has no pages to turn"
        );
        exercised.push("next_page (unreachable)");
    }

    eprintln!(
        "sync_observation_clock: {} outcome(s) exercised: {}",
        exercised.len(),
        exercised.join(", ")
    );
    assert!(
        exercised.len() >= 8,
        "fewer outcomes than SyncOutcome has variants: {exercised:?}"
    );
}

/// §21.9's rule 2: a value and its clock commit in **one transaction**. A 403 writes
/// `permitted = 0` and dates nothing — the read observed the *access* state and nothing else.
#[test]
fn a_403_records_the_access_state_and_still_dates_nothing() {
    let f = fixture();
    let before = seeded(&f);
    f.clock.set_unix(LATER);
    f.transport.push(response(403, &[], Vec::new()));
    run_project_remote(&f.deps, &f.index, f.project).expect("ran");

    let after = read_row(&f.index);
    assert_eq!(after.permitted, 0, "the access state was observed");
    assert_eq!(before.permitted, 1);
    assert_eq!(
        after.observed_at, before.observed_at,
        "and dating the counts by a read that returned none of them is the marker lying"
    );
}

/// **R65's ownership, as a structure rather than a sentence.** `core/src/sync/` writes **no**
/// `remote_*` row of its own: §25.7 owns the tables' shape, so it owns what writes them, and a
/// second writer for another plan's rows is R1's shape — it compiles, it passes its own tests,
/// and it diverges the first time the two disagree about what a 304 writes.
#[test]
fn the_sync_module_declares_no_writer_of_another_plans_tables() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/sync");
    let mut scanned = 0_usize;
    let mut offenders: Vec<String> = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("core/src/sync is readable") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            scanned += 1;
            let source = std::fs::read_to_string(&path).expect("readable");
            let upper = source.to_uppercase();
            for table in ["REMOTE_REPO", "REMOTE_TOPIC", "REMOTE_CI_RUN"] {
                for verb in ["INSERT INTO", "UPDATE"] {
                    let needle = format!("{verb} {table}");
                    if upper.contains(&needle) {
                        offenders.push(format!("{}: {needle}", path.display()));
                    }
                }
            }
        }
    }
    eprintln!("sync_observation_clock: scanned {scanned} file(s) under core/src/sync");
    assert!(
        scanned > 0,
        "a gate whose passing run scans zero files is a failing gate"
    );
    assert!(
        offenders.is_empty(),
        "core/src/sync writes a table §25.7 owns: {offenders:?}"
    );
}

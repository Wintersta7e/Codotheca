#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! `account_repos` — **the page is the transaction** — and its cadence.
//!
//! Driven through the **real `GitHubProvider` over a `FakeTransport`**, so the JSON, the `Link`
//! pagination and the admission pass are the product's own and not a stand-in for them. §21.9's
//! rule 2 is what the per-page transaction exists for: a listing whose values landed and whose
//! clock did not is the currency invariant broken.
//!
//! Covers **AC-P2-21-15** and **AC-P2-22-7's wire half**.

use std::sync::{Arc, Mutex};

use codotheca_core::accounts::keychain::{token_ref, SecretToken, TokenStore};
use codotheca_core::accounts::store::{insert_account, upsert_orgs, NewAccount};
use codotheca_core::http::{HttpResponse, HttpTransport, TransportError};
use codotheca_core::index::Index;
use codotheca_core::protocol::{AccountId, AuthKind, ScopeTier, SyncTaskKind};
use codotheca_core::provider::listing::OrgListing;
use codotheca_core::provider::GitHubProvider;
use codotheca_core::sync::http::ObservingTransport;
use codotheca_core::sync::outcome::SyncOutcome;
use codotheca_core::sync::schedule::{due_listings, LISTING_INTERVAL_SECS};
use codotheca_core::sync::state::{apply_outcome, SyncTaskStateRow};
use codotheca_core::sync::store::put;
use codotheca_core::sync::tasks::rename::run_rename_probe;
use codotheca_core::sync::tasks::repos::run_account_repos;
use codotheca_core::sync::SyncDeps;
use codotheca_core::testing::{FakeClock, FakeTokenStore, FakeTransport, TempIndex};

const NOW: i64 = 1_800_000_000;
const HOST: &str = "forge.example.invalid";
/// The **repair** reads `crate::provider::declared_host_aliases()`, and the one shipped adapter
/// declares a single canonical host (`core/src/provider/listing.rs:4-9`). So a project the probe
/// can repair carries that host in its `remote_key`, whatever host the transport is pointed at —
/// the two are different questions and the fixtures keep them apart deliberately.
const PROBE_HOST: &str = "github.com";

struct Fixture {
    index: Arc<Mutex<Index>>,
    transport: Arc<FakeTransport>,
    deps: SyncDeps,
    account: AccountId,
    _dir: tempfile::TempDir,
}

/// One repository as the forge's listing endpoint renders it.
fn repo_json(id: u64, owner: &str, name: &str, push: Option<bool>) -> String {
    let permissions = match push {
        Some(push) => format!(r#","permissions":{{"push":{push}}}"#),
        // **No permission object at all** — §20's rule is that this is *unknown*, never false.
        None => String::new(),
    };
    format!(
        r#"{{"id":{id},"clone_url":"https://{HOST}/{owner}/{name}.git",
            "owner":{{"login":"{owner}","type":"User"}},"name":"{name}",
            "fork":false,"archived":false,"private":false{permissions}}}"#
    )
}

fn page(body: &str, next: Option<&str>) -> HttpResponse {
    let mut headers: Vec<(&str, &str)> = vec![("x-ratelimit-resource", "core")];
    let link;
    if let Some(next) = next {
        link = format!(r#"<{next}>; rel="next""#);
        headers.push(("link", &link));
    }
    HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers(headers),
        body: body.as_bytes().to_vec(),
    }
}

fn fixture() -> Fixture {
    let temp = TempIndex::new();
    let dir = tempfile::tempdir().expect("tmp");
    let mut index = Index::open_at(dir.path(), NOW).expect("index");
    drop(temp);

    let account = index
        .with_tx(|tx| {
            let id = insert_account(
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
            upsert_orgs(
                tx,
                id,
                &[OrgListing {
                    login: "anorg".to_owned(),
                    repo_count_seen: None,
                }],
                NOW,
            )
            .expect("orgs");
            Ok(id)
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
        .expect("token stored");
    let provider = Arc::new(GitHubProvider::new(
        Arc::clone(&observing) as Arc<dyn HttpTransport>,
        HOST.to_owned(),
    ));

    Fixture {
        index: Arc::new(Mutex::new(index)),
        transport,
        deps: SyncDeps {
            provider,
            transport: observing,
            tokens,
            clock,
            cancel: codotheca_core::cancel::CancelToken::new(),
        },
        account,
        _dir: dir,
    }
}

/// **AC-P2-21-15.** An entry with **no permission object** settles counted in
/// `skippedUnknownPermission`, admitted **nowhere**, and dropped **silently nowhere**.
///
/// p2-20's rule is that a listing entry with no permission object is *unknown, not false*, and
/// its count must be **visible rather than absent** — silent suppression is the failure §11.1
/// forbids by name. The five figures are asserted to account for every entry the page carried.
#[test]
fn an_entry_with_no_permission_object_is_counted_and_admitted_nowhere() {
    let f = fixture();
    f.transport.push(page(
        &format!(
            "[{},{},{}]",
            repo_json(1, "owner", "alpha", Some(true)),
            repo_json(2, "owner", "beta", None),
            repo_json(3, "owner", "gamma", Some(false))
        ),
        None,
    ));

    let (outcome, summary) =
        run_account_repos(&f.deps, &f.index, f.account, None).expect("the listing ran");
    assert_eq!(outcome, SyncOutcome::Done);
    eprintln!(
        "sync_listing: listed {} admitted {} unknown-permission {} ambiguous {} suppressed {}",
        summary.listed,
        summary.admitted,
        summary.skipped_unknown_permission,
        summary.ambiguous,
        summary.suppressed
    );
    assert_eq!(summary.skipped_unknown_permission, 1, "beta");
    assert_eq!(summary.admitted, 1, "alpha alone can push");
    assert_eq!(summary.listed, 1, "only admitted entries reach ingest");

    // Admitted nowhere: the project set is exactly the one admissible repository.
    let guard = f.index.lock().expect("index");
    let projects: Vec<String> = guard
        .conn()
        .prepare("SELECT name FROM project ORDER BY name")
        .expect("prepared")
        .query_map([], |r| r.get::<_, String>(0))
        .expect("query")
        .map(Result::unwrap)
        .collect();
    assert_eq!(projects, ["alpha"], "beta and gamma reached no row");
}

/// **The page is the transaction.** A three-page listing writes page 1's rows *before* page 2 is
/// requested, and a transport error on page 2 leaves page 1's rows **and** page 1's clock
/// committed, with the cursor preserved.
#[test]
fn a_page_commits_before_the_next_is_requested_and_a_later_failure_keeps_it() {
    let f = fixture();
    let next = format!("https://{HOST}/user/repos?page=2");
    f.transport.push(page(
        &format!("[{}]", repo_json(1, "owner", "alpha", Some(true))),
        Some(&next),
    ));
    f.transport.push_err(TransportError::Timeout);

    let (first, _) = run_account_repos(&f.deps, &f.index, f.account, None).expect("page 1");
    assert_eq!(
        first,
        SyncOutcome::NextPage {
            cursor: next.clone()
        }
    );
    {
        let guard = f.index.lock().expect("index");
        let rows: i64 = guard
            .conn()
            .query_row("SELECT count(*) FROM project", [], |r| r.get(0))
            .expect("counted");
        assert_eq!(rows, 1, "page 1 committed before page 2 was asked for");
    }

    let (second, _) =
        run_account_repos(&f.deps, &f.index, f.account, Some(&next)).expect("page 2 answered");
    assert!(
        matches!(second, SyncOutcome::TransientFail { .. }),
        "a timeout is transient: {second:?}"
    );
    let guard = f.index.lock().expect("index");
    let rows: i64 = guard
        .conn()
        .query_row("SELECT count(*) FROM project", [], |r| r.get(0))
        .expect("counted");
    assert_eq!(rows, 1, "page 1's rows survived page 2's failure");
    let touched: Option<i64> = guard
        .conn()
        .query_row("SELECT max(updated_at) FROM project", [], |r| r.get(0))
        .expect("read");
    assert_eq!(touched, Some(NOW), "and page 1's clock committed with them");
}

/// §21.7: the cursor is **resumption state** and is cleared on any settle that is not `NextPage`.
/// Leaving it behind would restart the next scheduled listing halfway through the last one.
#[test]
fn a_settle_that_is_not_a_next_page_clears_the_cursor() {
    let row = SyncTaskStateRow {
        cursor: Some("page-2".to_owned()),
        ..SyncTaskStateRow::queued(SyncTaskKind::AccountRepos, Some(1), NOW)
    };
    let (after, _) = apply_outcome(&row, &SyncOutcome::Done, NOW);
    assert_eq!(after.cursor, None);
    let (after, _) = apply_outcome(&row, &SyncOutcome::NotModified, NOW);
    assert_eq!(after.cursor, None);
    let (after, _) = apply_outcome(
        &row,
        &SyncOutcome::NextPage {
            cursor: "page-3".to_owned(),
        },
        NOW,
    );
    assert_eq!(
        after.cursor.as_deref(),
        Some("page-3"),
        "the one that keeps it"
    );
}

/// §21.5's cadence. `due_listings` returns an account whose last settle is older than six hours
/// and not one that is younger, and an account that has never listed is due at once.
#[test]
fn a_listing_is_due_six_hours_after_it_last_settled() {
    let f = fixture();
    assert_eq!(LISTING_INTERVAL_SECS, 6 * 60 * 60);

    {
        let guard = f.index.lock().expect("index");
        assert_eq!(
            due_listings(guard.conn(), NOW).expect("due"),
            vec![f.account],
            "an account that has never listed is due at once"
        );
    }

    let mut settled = SyncTaskStateRow::queued(SyncTaskKind::AccountRepos, Some(f.account.0), NOW);
    settled.state = codotheca_core::protocol::SyncTaskState::Ok;
    {
        let mut guard = f.index.lock().expect("index");
        guard.with_tx(|tx| put(tx, &settled)).expect("put");
    }

    let guard = f.index.lock().expect("index");
    assert!(
        due_listings(guard.conn(), NOW + LISTING_INTERVAL_SECS - 1)
            .expect("due")
            .is_empty(),
        "one second early is not due"
    );
    assert_eq!(
        due_listings(guard.conn(), NOW + LISTING_INTERVAL_SECS).expect("due"),
        vec![f.account],
        "and on the interval it is"
    );
}

/// **AC-P2-22-7's wire half.** A suppression must be reported **with the project that blocked it
/// named**, and a count cannot name one (R62).
///
/// `suppressedBy` is positionally aligned with the suppressions and **not deduplicated**: one
/// project blocking two entries reads as two, so `suppressedBy.len() == suppressed` is an
/// invariant, and that equality is what makes a dropped id visible.
#[test]
fn a_suppression_names_the_project_that_blocked_it_once_per_entry() {
    let f = fixture();
    // §22.3's suppressor: a **cloned** project whose stored `remote_key` has the same path and a
    // **different host**. It is not a link candidate — the folded keys differ — so the matcher
    // reaches the suppression arm, which is the case that must be reported rather than silently
    // creating a second tile for a repository the user already has under another host.
    {
        let mut guard = f.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, remote_key, created_at, updated_at)
                     VALUES ('blocker', 'blocker', 'other.example.invalid/owner/alpha', ?1, ?1)",
                    [NOW],
                )?;
                let project = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO location (project_id, kind, path_bytes, path_key, path_display,
                                           store_key, presence, repo_kind)
                     VALUES (?1, ?2, ?3, ?3, ?4, 'store', 'present', 'worktree')",
                    rusqlite::params![
                        project,
                        if cfg!(windows) { "win" } else { "linux" },
                        b"/blocker".to_vec(),
                        "/blocker"
                    ],
                )?;
                Ok(())
            })
            .expect("blocker");
    }

    f.transport.push(page(
        &format!(
            "[{},{}]",
            repo_json(11, "owner", "alpha", Some(true)),
            repo_json(12, "owner", "alpha", Some(true))
        ),
        None,
    ));
    let (_, summary) = run_account_repos(&f.deps, &f.index, f.account, None).expect("listed");

    eprintln!(
        "sync_listing: suppressed {} suppressed_by {:?}",
        summary.suppressed, summary.suppressed_by
    );
    assert_eq!(
        i64::try_from(summary.suppressed_by.len()).expect("small"),
        summary.suppressed,
        "a dropped id is invisible unless these two agree"
    );
    assert_eq!(summary.suppressed, 2, "both entries blocked");
    assert_eq!(
        summary.suppressed_by[0], summary.suppressed_by[1],
        "one project blocking two entries reads as two, not as one"
    );
}

/// A listing with nothing suppressed settles with an **empty list and a zero**, and the empty
/// list means *nothing was suppressed* — never *not computed*, which is why the count stays
/// beside it rather than being replaced by the list's length.
#[test]
fn a_listing_with_no_suppression_settles_with_an_empty_list_and_a_zero() {
    let f = fixture();
    f.transport.push(page(
        &format!("[{}]", repo_json(1, "owner", "alpha", Some(true))),
        None,
    ));
    let (_, summary) = run_account_repos(&f.deps, &f.index, f.account, None).expect("listed");
    assert_eq!(summary.suppressed, 0);
    assert!(summary.suppressed_by.is_empty());
    assert_eq!(
        i64::try_from(summary.suppressed_by.len()).expect("small"),
        summary.suppressed
    );
}

/// A rate-limited listing mirrors its headers and settles as a park, **writing no project row**:
/// §21.9's rule 1 — a call that did not observe a value must not move that value's clock, and a
/// 403 observed nothing about any repository.
#[test]
fn a_throttled_listing_writes_no_row_and_still_mirrors_its_budget() {
    let f = fixture();
    let reset = (NOW + 900).to_string();
    f.transport.push(HttpResponse {
        status: 403,
        headers: codotheca_core::http::normalise_headers([
            ("x-ratelimit-resource", "core"),
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", reset.as_str()),
        ]),
        body: Vec::new(),
    });

    let (outcome, summary) = run_account_repos(&f.deps, &f.index, f.account, None).expect("ran");
    assert!(
        matches!(outcome, SyncOutcome::Throttled { .. }),
        "{outcome:?}"
    );
    assert_eq!(summary.listed, 0);

    let guard = f.index.lock().expect("index");
    let projects: i64 = guard
        .conn()
        .query_row("SELECT count(*) FROM project", [], |r| r.get(0))
        .expect("counted");
    assert_eq!(projects, 0);
    let remaining: Option<i64> = guard
        .conn()
        .query_row(
            "SELECT remaining FROM sync_budget WHERE account_id = ?1",
            [f.account.0],
            |r| r.get(0),
        )
        .expect("the budget row exists");
    assert_eq!(remaining, Some(0), "the 403's headers reached the mirror");
}

/// **R97(c), read from the other side.** `REMOTE_STALE_AFTER_SECS` has exactly one declaration
/// and this plan **reads** it rather than declaring a second.
///
/// The two are not the same decision — one is a scheduling cadence in the core, the other a
/// rendering threshold in the renderer — but the renderer's own doc derives its number from this
/// cadence in terms (*"six hours is §21.5's own `account_repos` cadence, so a remote observation
/// older than it has missed at least one scheduled read"*). That makes the citation load-bearing,
/// and R24's rule applies: **a cross-language mirror needs a test that reads the other side.**
/// Change the cadence without the threshold and this says so.
#[test]
fn the_renderers_staleness_threshold_still_cites_this_cadence() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../app/src/renderer/derive/observation.ts");
    let source = std::fs::read_to_string(&path).expect("the renderer module is readable");
    assert!(!source.is_empty(), "an empty read proves nothing");

    let marker = "export const REMOTE_STALE_AFTER_SECS = ";
    let start = source
        .find(marker)
        .expect("REMOTE_STALE_AFTER_SECS has exactly one declaration, and this is it")
        + marker.len();
    let literal: String = source[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '_')
        .filter(|c| *c != '_')
        .collect();
    let declared: i64 = literal.parse().expect("a numeric literal");
    eprintln!(
        "sync_listing: renderer threshold {declared}s, core cadence {LISTING_INTERVAL_SECS}s"
    );
    assert_eq!(
        declared, LISTING_INTERVAL_SECS,
        "the renderer derives its staleness threshold from this cadence; moving one without the \
         other makes that derivation false"
    );
    assert_eq!(
        source.matches(marker).count(),
        1,
        "one owner, and this plan is not a second"
    );
}

// ---------------------------------------------------------------------------------------------
// §21.3's third task: `rename_probe`.
// ---------------------------------------------------------------------------------------------

/// A renamed repository keeps its stable forge id and loses its path, so §22.1's second basis
/// fails. The probe asks what the stored path resolves to **once** and writes the id, after which
/// the match is basis **(a)** forever.
#[test]
fn a_rename_probe_writes_the_stable_id_and_settles_the_basis() {
    let f = fixture();
    {
        let mut guard = f.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, remote_key, created_at, updated_at)
                     VALUES ('renamed', 'renamed', ?1, ?2, ?2)",
                    rusqlite::params![format!("{PROBE_HOST}/owner/old-name"), NOW],
                )?;
                Ok(())
            })
            .expect("unmatched project");
    }
    // The forge resolves the old path to the repository's current identity.
    f.transport.push(HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
        body: repo_json(4242, "owner", "new-name", Some(true)).into_bytes(),
    });

    let outcome = run_rename_probe(&f.deps, &f.index, f.account).expect("probed");
    assert_eq!(outcome, SyncOutcome::Done);

    let guard = f.index.lock().expect("index");
    let (id, basis): (Option<String>, Option<String>) = guard
        .conn()
        .query_row(
            "SELECT provider_repo_id, remote_link_basis FROM project WHERE name = 'renamed'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(id.as_deref(), Some("4242"), "the stable id, not the path");
    assert_eq!(basis.as_deref(), Some("provider_id"), "basis (a), forever");
}

/// **A `404` here is `NotFound`, and `NotFound` is `ok`** — never `blocked`, never a deletion.
/// The repository is *unseen*, never *gone*: absence is not evidence, and no row is deleted.
#[test]
fn a_probe_that_answers_not_found_deletes_nothing_and_does_not_block() {
    let f = fixture();
    {
        let mut guard = f.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, remote_key, description,
                                          created_at, updated_at)
                     VALUES ('gone', 'gone', ?1, 'still here', ?2, ?2)",
                    rusqlite::params![format!("{PROBE_HOST}/owner/gone"), NOW],
                )?;
                Ok(())
            })
            .expect("unmatched project");
    }
    f.transport.push(HttpResponse {
        status: 404,
        headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
        body: Vec::new(),
    });

    let outcome = run_rename_probe(&f.deps, &f.index, f.account).expect("probed");
    let (after, _) = apply_outcome(
        &SyncTaskStateRow::queued(SyncTaskKind::RenameProbe, Some(f.account.0), NOW),
        &outcome,
        NOW,
    );
    assert_eq!(
        after.state,
        codotheca_core::protocol::SyncTaskState::Ok,
        "a 404 settles ok: {outcome:?}"
    );

    let guard = f.index.lock().expect("index");
    let (rows, description): (i64, Option<String>) = guard
        .conn()
        .query_row(
            "SELECT count(*), max(description) FROM project WHERE name = 'gone'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("row");
    assert_eq!(rows, 1, "no row is deleted by a probe");
    assert_eq!(
        description.as_deref(),
        Some("still here"),
        "and none is cleared"
    );
}

/// **The probe is not re-queued on every listing.** Once a project carries `provider_repo_id`
/// with basis (a) it is never a candidate again, which is what stops one unresolvable rename
/// costing a request per listing forever.
#[test]
fn a_project_that_already_carries_its_id_is_never_probed_again() {
    let f = fixture();
    {
        let mut guard = f.index.lock().expect("index");
        guard
            .with_tx(|tx| {
                tx.execute(
                    "INSERT INTO project (name, seed_basename, remote_key, created_at, updated_at)
                     VALUES ('renamed', 'renamed', ?1, ?2, ?2)",
                    rusqlite::params![format!("{PROBE_HOST}/owner/old-name"), NOW],
                )?;
                Ok(())
            })
            .expect("unmatched project");
    }
    f.transport.push(HttpResponse {
        status: 200,
        headers: codotheca_core::http::normalise_headers([("x-ratelimit-resource", "core")]),
        body: repo_json(4242, "owner", "new-name", Some(true)).into_bytes(),
    });
    run_rename_probe(&f.deps, &f.index, f.account).expect("first probe");
    let spent = f.transport.request_count();
    assert_eq!(spent, 1, "exactly one lookup for one unmatched key");

    // A second probe finds nothing to ask about and spends nothing.
    run_rename_probe(&f.deps, &f.index, f.account).expect("second probe");
    assert_eq!(
        f.transport.request_count(),
        spent,
        "a resolved project must never cost a second request"
    );
}

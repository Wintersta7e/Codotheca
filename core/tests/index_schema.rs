// The shared helpers below are `pub` so later tasks appending to this file can reuse them.
// `must_use_candidate` and `missing_panics_doc` are about a documented public API; in a test
// binary nothing outside the file can call them and a panic is how a test reports failure.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::must_use_candidate,
    clippy::missing_panics_doc
)]
//! The migrated schema: every table's columns, constraints and indexes, as the spec names them.

use codotheca_core::index::migrate::{apply_all, MIGRATIONS, SUPPORTED_SCHEMA_VERSION};
use codotheca_core::index::{open_connection, Index};

/// A database with every shipped migration applied, and the directory that holds it.
pub fn fresh() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let mut conn = open_connection(&Index::db_path(dir.path())).unwrap();
    let reached = apply_all(&mut conn, MIGRATIONS).unwrap();
    assert_eq!(reached, SUPPORTED_SCHEMA_VERSION);
    (dir, conn)
}

/// Whether `sqlite_master` holds exactly one object of `kind` (`table`, `index`) named `name`.
pub fn has(conn: &rusqlite::Connection, kind: &str, name: &str) -> bool {
    let n: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type=?1 AND name=?2",
            [kind, name],
            |r| r.get(0),
        )
        .unwrap();
    n == 1
}

/// `table`'s column names, in declaration order.
pub fn columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

/// A minimal project row: everything NOT NULL with no default, and nothing else.
pub fn insert_project(conn: &rusqlite::Connection, name: &str) -> i64 {
    conn.execute(
        "INSERT INTO project (name, seed_basename, created_at, updated_at)
         VALUES (?1, ?1, 1, 1)",
        [name],
    )
    .unwrap();
    conn.last_insert_rowid()
}

#[test]
fn migration_0001_creates_its_four_tables() {
    let (_d, conn) = fresh();
    for t in ["app_meta", "view_state", "project", "project_redirect"] {
        assert!(has(&conn, "table", t), "{t} is missing");
    }
}

#[test]
fn project_carries_every_v2_2_column_v2_1_left_out() {
    let (_d, conn) = fresh();
    let cols = columns(&conn, "project");
    for c in [
        "seed_basename",
        "reroll_offset",
        "acknowledged_at",
        "association_kind",
        "merged_into",
        "ambiguous_lineage",
        "condition_signal",
        "condition_material",
        "completion_lit",
        "completion_applicable",
        "slow_repo",
        "error_kind",
        "error_detail",
        "error_at",
        "notes",
    ] {
        assert!(cols.iter().any(|x| x == c), "project.{c} is missing");
    }
}

#[test]
fn completion_has_no_default_and_is_null_on_a_fresh_row() {
    let (_d, conn) = fresh();
    let id = insert_project(&conn, "thing");

    let (lit, applicable): (Option<i64>, Option<i64>) = conn
        .query_row(
            "SELECT completion_lit, completion_applicable FROM project WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(lit, None, "a default of 0 would render unknown as zero");
    assert_eq!(applicable, None);

    // The pragma is the direct evidence: dflt_value must be NULL for both.
    for c in ["completion_lit", "completion_applicable"] {
        let dflt: Option<String> = conn
            .query_row(
                "SELECT dflt_value FROM pragma_table_info('project') WHERE name=?1",
                [c],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(dflt, None, "{c} must have no DEFAULT (§1.10)");
    }
}

#[test]
fn the_schema_refuses_a_lit_count_with_no_applicable_count() {
    let (_d, conn) = fresh();
    let id = insert_project(&conn, "thing");

    let half = conn.execute("UPDATE project SET completion_lit = 0 WHERE id = ?1", [id]);
    assert!(
        half.is_err(),
        "0 lit with NULL applicable is exactly the unknown-as-zero shape §1.10 forbids"
    );

    let zero_applicable = conn.execute(
        "UPDATE project SET completion_lit = 0, completion_applicable = 0 WHERE id = ?1",
        [id],
    );
    assert!(
        zero_applicable.is_err(),
        "0 of 0 is not a computed completion"
    );

    let over = conn.execute(
        "UPDATE project SET completion_lit = 11, completion_applicable = 10 WHERE id = ?1",
        [id],
    );
    assert!(over.is_err(), "lit may not exceed applicable");

    // A real zero — nought of ten lit — is legal and is the case the invariant protects.
    conn.execute(
        "UPDATE project SET completion_lit = 0, completion_applicable = 10 WHERE id = ?1",
        [id],
    )
    .unwrap();
}

/// A4's structural half. The `\bblueprint\b` grep leaves `c58-design-band-names`; what replaces
/// it is this — the DDL `CHECK` refusing `blueprint` as a `condition_signal` **value**, which is
/// what criterion 58 was ever about and is stronger than a text search over three directories.
///
/// The accepted count is printed and asserted, so a run that inserted nothing is a failing run
/// rather than a quiet pass over an empty loop.
#[test]
fn condition_signal_takes_only_section_5_4_words() {
    let (_d, conn) = fresh();
    let id = insert_project(&conn, "thing");
    let mut accepted = 0_usize;
    for good in [
        "live",
        "idle",
        "dormant",
        "neglected",
        "abandoned",
        "offline",
        "empty",
    ] {
        conn.execute(
            "UPDATE project SET condition_signal = ?2 WHERE id = ?1",
            rusqlite::params![id, good],
        )
        .unwrap();
        accepted += 1;
    }
    // stderr, never stdout: `print_stdout` is denied crate-wide (core/Cargo.toml).
    eprintln!("condition_signal: {accepted} of §5.4a's slugs accepted by the column");
    assert_eq!(
        accepted, 7,
        "§5.4a has seven slugs; a run that accepted {accepted} proves nothing"
    );
    let mut refused = 0_usize;
    for banned in ["warm", "cooling", "blueprint"] {
        let r = conn.execute(
            "UPDATE project SET condition_signal = ?2 WHERE id = ?1",
            rusqlite::params![id, banned],
        );
        assert!(
            r.is_err(),
            "{banned} is the design's vocabulary, not §5.4's"
        );
        refused += 1;
    }
    eprintln!("condition_signal: {refused} band words refused by the CHECK");
    assert_eq!(refused, 3, "warm, cooling and blueprint are all three");

    // The second structural refusal A4 names: the generated union type. Both halves are
    // asserted here so the narrowing loses nothing when the grep pattern goes.
    for banned in ["warm", "cooling", "blueprint"] {
        let decoded: Result<codotheca_core::protocol::ConditionSignal, _> =
            serde_json::from_value(serde_json::Value::String(banned.to_owned()));
        assert!(
            decoded.is_err(),
            "{banned} must not decode as a ConditionSignal"
        );
    }
    for good in [
        "live",
        "idle",
        "dormant",
        "neglected",
        "abandoned",
        "offline",
        "empty",
    ] {
        let decoded: Result<codotheca_core::protocol::ConditionSignal, _> =
            serde_json::from_value(serde_json::Value::String(good.to_owned()));
        assert!(decoded.is_ok(), "{good} is one of §5.4a's seven");
    }
}

#[test]
fn project_ids_are_autoincrement_so_a_tombstone_can_never_be_aliased() {
    let (_d, conn) = fresh();
    let first = insert_project(&conn, "a");
    conn.execute("DELETE FROM project WHERE id = ?1", [first])
        .unwrap();
    let second = insert_project(&conn, "b");
    assert!(
        second > first,
        "AUTOINCREMENT is required by §1.5 so a merged-away rowid is never reused"
    );
}

#[test]
fn the_four_project_indexes_of_section_1_11_exist() {
    let (_d, conn) = fresh();
    for idx in [
        "idx_project_lineage",
        "idx_project_remote",
        "idx_project_shelf_order",
        "idx_project_reference_order",
    ] {
        assert!(has(&conn, "index", idx), "{idx} is missing");
    }
}

#[test]
fn app_meta_is_seeded_with_the_keys_section_1_9_names() {
    let (_d, conn) = fresh();
    let v: String = conn
        .query_row(
            "SELECT v FROM app_meta WHERE k='sidecar_generation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v, "0");
}

use codotheca_core::index::path::{PathPlatform, StoredPath};

// R1: this is a TEST HELPER. There is no production `location` writer in plan 04; plan 08 owns
// it, written in the same transaction as `resolve_identity`.
/// One `location` row for `project_id` at `raw`, returning its id.
///
/// `raw` is read as a path on `kind`'s platform, and `distro` is written only when given, so a
/// test can leave the column to its default.
///
/// # Errors
///
/// The `INSERT`'s own error: the tests here reach the CHECK and UNIQUE constraints through it.
pub fn insert_location(
    conn: &rusqlite::Connection,
    project_id: i64,
    kind: &str,
    distro: Option<&str>,
    raw: &[u8],
) -> rusqlite::Result<i64> {
    let platform = if kind == "win" {
        PathPlatform::Windows
    } else {
        PathPlatform::Unix
    };
    let p = StoredPath::from_bytes(raw.to_vec(), platform);
    let (bytes, key, display) = p.as_params();
    let mut sql = String::from(
        "INSERT INTO location
           (project_id, kind, path_bytes, path_key, path_display,
            volume_key, store_key, presence, repo_kind",
    );
    if distro.is_some() {
        sql.push_str(", distro");
    }
    // [R27] `repo_kind` is NOT NULL with no default, deliberately: a fixture must say which
    // kind it is rather than inherit one. This helper picks 'worktree' visibly; production
    // writes go through plan 08's `upsert_location`, which takes it from the caller.
    sql.push_str(") VALUES (?1, ?2, ?3, ?4, ?5, 'vol', 'store', 'present', 'worktree'");
    if distro.is_some() {
        sql.push_str(", ?6");
    }
    sql.push(')');
    match distro {
        Some(d) => conn.execute(
            &sql,
            rusqlite::params![project_id, kind, bytes, key, display, d],
        )?,
        None => conn.execute(
            &sql,
            rusqlite::params![project_id, kind, bytes, key, display],
        )?,
    };
    Ok(conn.last_insert_rowid())
}

/// The five worktree facts §6 keeps NULL until they are actually observed.
type ObservedCounts = (
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

#[test]
fn migration_0002_creates_its_tables_and_indexes() {
    let (_d, conn) = fresh();
    for t in ["location", "scan_root", "submodule_edge"] {
        assert!(has(&conn, "table", t), "{t} is missing");
    }
    for i in ["idx_location_project", "idx_location_store_presence"] {
        assert!(has(&conn, "index", i), "{i} is missing");
    }
}

#[test]
fn location_carries_the_v2_2_columns_and_leaves_worktree_facts_null() {
    let (_d, conn) = fresh();
    let cols = columns(&conn, "location");
    for c in [
        "head_oid",
        "fetch_head_at",
        "trusted_at",
        "refstate_observed_at",
        "refstate_basis",
        "worktree_observed_at",
        "repo_kind",
        "common_dir_bytes",
    ] {
        assert!(cols.iter().any(|x| x == c), "location.{c} is missing");
    }

    let p = insert_project(&conn, "thing");
    let l = insert_location(&conn, p, "linux", None, b"/home/u/thing").unwrap();
    let (dirty, untracked, ahead, behind, fetch): ObservedCounts = conn
        .query_row(
            "SELECT is_dirty, untracked_count, ahead, behind, fetch_head_at
             FROM location WHERE id=?1",
            [l],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(
        dirty, None,
        "a default of 0 would read as verified clean (§6)"
    );
    assert_eq!(untracked, None);
    assert_eq!(ahead, None);
    assert_eq!(behind, None);
    assert_eq!(fetch, None, "NULL means no fetch recorded, never 0 (§1.3)");
}

#[test]
fn distro_defaults_to_empty_so_the_unique_constraint_actually_bites() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");

    insert_location(&conn, p, "linux", None, b"/home/u/thing").unwrap();
    let second = insert_location(&conn, p, "linux", None, b"/home/u/thing");
    assert!(
        second.is_err(),
        "one path indexed twice is acceptance criterion 1 failing"
    );

    let stored: String = conn
        .query_row(
            "SELECT distro FROM location WHERE project_id=?1",
            [p],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, "", "not NULL — that was the v1 defect");
}

/// The defect, demonstrated rather than described: with a NULL-able `distro` the identical
/// pair of inserts succeeds, because SQLite treats NULLs as distinct in a UNIQUE index.
#[test]
fn the_v1_shape_with_a_nullable_distro_admits_the_duplicate() {
    let (_d, conn) = fresh();
    conn.execute_batch(
        "CREATE TABLE v1_location (
           kind TEXT NOT NULL, distro TEXT, path_key BLOB NOT NULL,
           UNIQUE (kind, distro, path_key));",
    )
    .unwrap();
    for _ in 0..2 {
        conn.execute(
            "INSERT INTO v1_location (kind, distro, path_key) VALUES ('linux', NULL, ?1)",
            [&b"/home/u/thing"[..]],
        )
        .unwrap();
    }
    let n: i64 = conn
        .query_row("SELECT count(*) FROM v1_location", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2, "this is what the NOT NULL default prevents");
}

#[test]
fn a_non_wsl_location_may_not_carry_a_distro() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");
    let bad = insert_location(&conn, p, "linux", Some("ubuntu"), b"/home/u/thing");
    assert!(bad.is_err(), "distro is meaningful only when kind = 'wsl'");
    insert_location(&conn, p, "wsl", Some("ubuntu"), b"/home/u/thing").unwrap();
}

#[test]
fn presence_takes_only_the_four_words_v2_1_fixed() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");
    let l = insert_location(&conn, p, "linux", None, b"/home/u/thing").unwrap();
    for good in ["present", "offline", "missing", "unscanned"] {
        conn.execute(
            "UPDATE location SET presence=?2 WHERE id=?1",
            rusqlite::params![l, good],
        )
        .unwrap();
    }
    assert!(conn
        .execute("UPDATE location SET presence='gone' WHERE id=?1", [l])
        .is_err());
}

#[test]
fn migration_0003_creates_its_tables_and_index() {
    let (_d, conn) = fresh();
    for t in [
        "identity",
        "identity_alias",
        "project_committer",
        "merge_record",
        "xp_events",
        "health_delta",
        "fts_commits",
    ] {
        assert!(has(&conn, "table", t), "{t} is missing");
    }
    assert!(has(&conn, "index", "idx_xp_events_project_ts"));
}

#[test]
fn fts_commits_is_a_plain_table_because_fts5_is_phase_3() {
    let (_d, conn) = fresh();
    let sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='fts_commits'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        !sql.to_ascii_lowercase().contains("virtual"),
        "§1.9 defers FTS5 to phase 3: {sql}"
    );
}

#[test]
fn an_identity_is_seeded_unconfirmed_and_confirmed_at_is_the_durable_record() {
    let (_d, conn) = fresh();
    conn.execute(
        "INSERT INTO identity (email, name, source) VALUES ('a@example.invalid', 'A', 'gitconfig')",
        [],
    )
    .unwrap();
    let confirmed: Option<i64> = conn
        .query_row("SELECT confirmed_at FROM identity", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        confirmed, None,
        "NULL is what records that the set is seeded but unconfirmed (§1.4)"
    );

    assert!(
        conn.execute(
            "INSERT INTO identity (email, source) VALUES ('A@EXAMPLE.INVALID', 'manual')",
            [],
        )
        .is_err(),
        "addresses are compared case-insensitively"
    );
    assert!(conn
        .execute(
            "INSERT INTO identity (email, source) VALUES ('b@example.invalid', 'guessed')",
            [],
        )
        .is_err());
}

#[test]
fn xp_event_track_and_kind_cannot_disagree() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");

    conn.execute(
        "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
         VALUES (1, ?1, 'lineage:abc|remote:h/o/n', 'commit_day',
                 'commit_day:abc:h/o/n:2026-08-21', 'git')",
        [p],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
         VALUES (2, ?1, 'lineage:abc|remote:h/o/n', 'session', 'session:7', 'session')",
        [p],
    )
    .unwrap();

    let crossed = conn.execute(
        "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
         VALUES (3, ?1, 'x', 'commit_day', 'commit_day:x', 'session')",
        [p],
    );
    assert!(
        crossed.is_err(),
        "a commit_day is git-derived; calling it session-derived would exempt it from recompute"
    );

    let duplicate = conn.execute(
        "INSERT INTO xp_events (ts, project_id, subject_key, kind, dedupe_key, track)
         VALUES (4, ?1, 'lineage:abc|remote:h/o/n', 'commit_day',
                 'commit_day:abc:h/o/n:2026-08-21', 'git')",
        [p],
    );
    assert!(duplicate.is_err(), "dedupe_key is UNIQUE");
}

/// [p3] §34.3's rebuilt table: a `DecayLayer` word and a known provenance are accepted, and each
/// CHECK refuses on its own — the refused rows differ from the accepted one in one column apiece,
/// so neither assertion can pass on the other's rule.
#[test]
fn health_delta_takes_a_decay_layer_and_a_known_provenance() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");
    conn.execute(
        "INSERT INTO health_delta (project_id, ts, layer, from_value, to_value, detected_in)
         VALUES (?1, 1, 'dust', 2.0, 1.0, 'background')",
        [p],
    )
    .unwrap();
    assert!(conn
        .execute(
            "INSERT INTO health_delta (project_id, ts, layer, detected_in)
             VALUES (?1, 2, 'dust', 'elsewhere')",
            [p],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO health_delta (project_id, ts, layer, detected_in)
             VALUES (?1, 3, 'roof', 'background')",
            [p],
        )
        .is_err());
}

#[test]
fn migration_0004_creates_its_tables_indexes_and_trigger() {
    let (_d, conn) = fresh();
    for t in [
        "session",
        "session_segment",
        "launch_target",
        "collection",
        "collection_member",
    ] {
        assert!(has(&conn, "table", t), "{t} is missing");
    }
    for i in [
        "idx_session_project_started",
        "idx_launch_target_project",
        "idx_collection_member_project",
    ] {
        assert!(has(&conn, "index", i), "{i} is missing");
    }
    assert!(has(
        &conn,
        "trigger",
        "collection_member_rejects_query_collection"
    ));
}

#[test]
fn an_open_session_may_not_carry_a_close_reason() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");
    conn.execute(
        "INSERT INTO session (project_id, started_at) VALUES (?1, 100)",
        [p],
    )
    .unwrap();
    let s = conn.last_insert_rowid();

    assert!(conn
        .execute("UPDATE session SET close_reason='stop' WHERE id=?1", [s])
        .is_err());
    conn.execute(
        "UPDATE session SET ended_at=200, close_reason='stop', credited_seconds=60 WHERE id=?1",
        [s],
    )
    .unwrap();
    assert!(conn
        .execute(
            "UPDATE session SET close_reason='vanished' WHERE id=?1",
            [s]
        )
        .is_err());
}

#[test]
fn launch_target_scopes_by_the_triple_and_has_no_is_default_column() {
    let (_d, conn) = fresh();
    let cols = columns(&conn, "launch_target");
    for c in ["project_id", "location_id", "language", "sort_index"] {
        assert!(cols.iter().any(|x| x == c), "launch_target.{c} is missing");
    }
    assert!(
        !cols.iter().any(|x| x == "is_default"),
        "§4bis.2a: the candidate list and the default are the same ordered list"
    );

    // The global tier: all three scope columns NULL.
    conn.execute(
        "INSERT INTO launch_target (kind, name, exec_bytes, sort_index)
         VALUES ('editor', 'an editor', ?1, 0)",
        [&b"/usr/bin/an-editor"[..]],
    )
    .unwrap();
    let (verify, cwd): (String, String) = conn
        .query_row(
            "SELECT verify_state, cwd_mode FROM launch_target",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(verify, "unverified", "a fresh row has not been verified");
    assert_eq!(cwd, "location");
}

#[test]
fn a_query_collection_owns_no_members() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");

    conn.execute(
        "INSERT INTO collection (name, kind, query_text, query_grammar_version)
         VALUES ('Dirty', 'query', 'is:dirty', 1)",
        [],
    )
    .unwrap();
    let q = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO collection (name, kind) VALUES ('By hand', 'manual')",
        [],
    )
    .unwrap();
    let m = conn.last_insert_rowid();

    conn.execute(
        "INSERT INTO collection_member (collection_id, project_id) VALUES (?1, ?2)",
        [m, p],
    )
    .unwrap();
    let refused = conn.execute(
        "INSERT INTO collection_member (collection_id, project_id) VALUES (?1, ?2)",
        [q, p],
    );
    assert!(refused.is_err(), "§1.9: a kind='query' row owns no members");

    assert!(
        conn.execute(
            "INSERT INTO collection (name, kind) VALUES ('BY HAND', 'manual')",
            []
        )
        .is_err(),
        "§8.8 renders a collection by name alone"
    );
    assert!(
        conn.execute(
            "INSERT INTO collection (name, kind) VALUES ('Broken', 'query')",
            []
        )
        .is_err(),
        "a query collection with no query is a chip that filters nothing"
    );
}

#[test]
fn migration_0005_creates_its_tables_and_index() {
    let (_d, conn) = fresh();
    for t in [
        "project_job_state",
        "art_scene",
        "peek_cache",
        "scan_run",
        "scan_problem",
    ] {
        assert!(has(&conn, "table", t), "{t} is missing");
    }
    assert!(has(&conn, "index", "idx_scan_problem_run_kind"));
}

#[test]
fn job_state_is_per_project_per_job_and_carries_a_cursor() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");
    conn.execute(
        "INSERT INTO project_job_state (project_id, job, state, at, cursor)
         VALUES (?1, 'j4', 'running', 10, 'commit:abcdef')",
        [p],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO project_job_state (project_id, job, state, at)
         VALUES (?1, 'j2', 'deferred_slow', 11)",
        [p],
    )
    .unwrap();
    assert!(
        conn.execute(
            "INSERT INTO project_job_state (project_id, job, state, at)
             VALUES (?1, 'j4', 'queued', 12)",
            [p],
        )
        .is_err(),
        "one row per project per job"
    );
    assert!(conn
        .execute(
            "INSERT INTO project_job_state (project_id, job, state, at)
             VALUES (?1, 'j9', 'queued', 13)",
            [p],
        )
        .is_err());
}

#[test]
fn scan_problem_kinds_are_the_six_groups_that_read_this_table() {
    let (_d, conn) = fresh();
    conn.execute(
        "INSERT INTO scan_run (generation, started_at, mode, roots_json)
         VALUES (1, 100, 'full', '[]')",
        [],
    )
    .unwrap();
    let run = conn.last_insert_rowid();

    for kind in [
        "permission_denied",
        "untrusted_repo",
        "unreadable_repo",
        "clock_skew",
        "non_utf8_path",
        "offline_store",
    ] {
        conn.execute(
            "INSERT INTO scan_problem (scan_run_id, kind, detail) VALUES (?1, ?2, 'd')",
            rusqlite::params![run, kind],
        )
        .unwrap();
    }
    assert!(
        conn.execute(
            "INSERT INTO scan_problem (scan_run_id, kind) VALUES (?1, 'deferred_slow')",
            [run],
        )
        .is_err(),
        "§11.1: deferred-slow reads project_job_state, not this table"
    );
    assert!(
        conn.execute(
            "INSERT INTO scan_problem (scan_run_id, kind, count) VALUES (?1, 'untrusted_repo', 0)",
            [run],
        )
        .is_err(),
        "§11.1: count = 0 removes the group; a zero row must not be storable"
    );
}

#[test]
fn art_scene_and_peek_cache_are_one_row_per_project() {
    let (_d, conn) = fresh();
    let p = insert_project(&conn, "thing");
    conn.execute(
        "INSERT INTO art_scene (project_id, scene_hash, scene_json, schema_version, state)
         VALUES (?1, 'deadbeef', '{}', 1, 'ready')",
        [p],
    )
    .unwrap();
    assert!(conn
        .execute(
            "INSERT INTO art_scene (project_id, scene_hash, scene_json, schema_version, state)
             VALUES (?1, 'other', '{}', 1, 'ready')",
            [p],
        )
        .is_err());

    conn.execute(
        "INSERT INTO peek_cache (project_id, readme_excerpt, computed_at) VALUES (?1, 'hi', 5)",
        [p],
    )
    .unwrap();
    let commits: Option<String> = conn
        .query_row("SELECT recent_commits_json FROM peek_cache", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        commits, None,
        "an uncomputed Peek payload is absent, not empty"
    );
}

fn plan(conn: &rusqlite::Connection, sql: &str) -> String {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let rows = stmt.query_map([], |r| r.get::<_, String>(3)).unwrap();
    rows.map(Result::unwrap).collect::<Vec<_>>().join(" | ")
}

/// §1.11's own justification is that at 1,000 rows a missing index still benchmarks fast, so
/// these exist to give the acceptance numbers a query plan rather than an accident. A
/// name-exists assertion does not test that; `EXPLAIN QUERY PLAN` does.
#[test]
fn every_section_1_11_index_has_a_query_plan_behind_it() {
    let (_d, conn) = fresh();
    let cases: [(&str, &str); 11] = [
        (
            "idx_location_project",
            "SELECT id FROM location WHERE project_id = 1",
        ),
        (
            "idx_location_store_presence",
            "SELECT id FROM location WHERE store_key = 'k' AND presence = 'present'",
        ),
        (
            "idx_project_lineage",
            "SELECT id FROM project WHERE lineage_key = 'l'",
        ),
        (
            "idx_project_remote",
            "SELECT id FROM project WHERE remote_key = 'r'",
        ),
        (
            "idx_project_shelf_order",
            "SELECT id FROM project WHERE is_hidden = 0 ORDER BY last_touched_at DESC",
        ),
        (
            "idx_project_reference_order",
            "SELECT id FROM project WHERE is_reference = 0 ORDER BY last_touched_at DESC",
        ),
        (
            "idx_collection_member_project",
            "SELECT collection_id FROM collection_member WHERE project_id = 1",
        ),
        (
            "idx_session_project_started",
            "SELECT id FROM session WHERE project_id = 1 ORDER BY started_at DESC",
        ),
        (
            "idx_xp_events_project_ts",
            "SELECT id FROM xp_events WHERE project_id = 1 ORDER BY ts",
        ),
        (
            "idx_launch_target_project",
            "SELECT id FROM launch_target WHERE project_id = 1",
        ),
        (
            "idx_scan_problem_run_kind",
            "SELECT id FROM scan_problem WHERE scan_run_id = 1 AND kind = 'untrusted_repo'",
        ),
    ];
    for (index, sql) in cases {
        let detail = plan(&conn, sql);
        assert!(
            detail.contains(index),
            "{index} is not used by its own query; plan was: {detail}"
        );
    }
}

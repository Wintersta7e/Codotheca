#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! Acceptance: §25.7's observation clock and §25.1's read-back, plus §5.2's five-rung chain.
//!
//! Test names come from `acceptance/criteria.json`.
//!
//! **This file asserts what the reader REPORTS after each outcome**; `core/tests/remote_facts_read.rs`
//! asserts the same six outcomes against the stored columns. A column written correctly and read
//! back into the wrong state is a defect neither test alone can see.
//!
//! **It drives the writers directly, never a sync runner.** §21's `run_project_remote` is a
//! **wave-5** export (R65) and a wave-4 plan may not consume one. The six outcomes are the calls
//! each one makes: `200` → `write_repo_facts`; `304` → `confirm_repo_facts`; `403` and `404` →
//! `mark_not_permitted`; **throttled and skipped → no call at all**, which is the case a test
//! driving a runner would most easily fake into existence and is here simply the absence of a
//! write.

use codotheca_core::identity::binding::RemoteBinding;
use codotheca_core::index::Index;
use codotheca_core::protocol::{ProjectId, RemoteFactsState};
use codotheca_core::provider::RepoFactsPayload;
use codotheca_core::remote::facts::remote_facts;
use codotheca_core::remote::store::{confirm_repo_facts, mark_not_permitted, write_repo_facts};

const NOW: i64 = 1_781_179_200;

fn seeded() -> (tempfile::TempDir, Index) {
    let dir = tempfile::tempdir().expect("tempdir");
    let index = Index::open(dir.path()).expect("open");
    index
        .conn()
        .execute(
            "INSERT INTO project (id, name, seed_basename, remote_key, provider,
                                  provider_repo_id, remote_link_basis, last_touched_at,
                                  created_at, updated_at)
             VALUES (1, 'widget', 'widget', 'github.com/acme/widget', 'github', '909',
                     'provider_id', ?1, ?1, ?1)",
            rusqlite::params![NOW],
        )
        .expect("seed project");
    index
        .conn()
        .execute(
            "INSERT INTO account (provider, host, login, auth_kind, scope_tier, granted_scopes,
                                  token_ref, connected_at)
             VALUES ('github', 'github.com', 'someone', 'device', 'private', '[]',
                     'github:github.com:someone', ?1)",
            rusqlite::params![NOW],
        )
        .expect("account");
    (dir, index)
}

fn binding() -> RemoteBinding {
    RemoteBinding {
        provider: "github".to_owned(),
        provider_repo_id: "909".to_owned(),
        remote_link_basis: None,
    }
}

fn payload() -> RepoFactsPayload {
    RepoFactsPayload {
        visibility: Some("public".to_owned()),
        description: Some("a widget".to_owned()),
        fork_parent_remote_key: None,
        stars: Some(41),
        open_issues: Some(7),
        good_first_issues: Some(0),
        open_prs: Some(2),
        open_prs_from_user: Some(0),
        topics: vec!["rust".to_owned()],
    }
}

fn in_tx<F>(index: &mut Index, f: F)
where
    F: FnOnce(&rusqlite::Transaction<'_>),
{
    let tx = index.conn_mut().transaction().expect("transaction");
    f(&tx);
    tx.commit().expect("commit");
}

fn read(index: &Index) -> (RemoteFactsState, Option<i64>) {
    let facts = remote_facts(index.conn(), ProjectId(1))
        .expect("read")
        .expect("a key produces facts");
    (facts.state, facts.observed_at)
}

/// **AC-P2-25-24.** Six outcomes, in sequence, asserted through the **reader**.
#[test]
fn ac_p2_25_24_the_clock_moves_only_for_a_read_that_observed_something() {
    let (_dir, mut index) = seeded();
    let mut exercised = 0_u32;

    // 200 — the value and its clock together.
    in_tx(&mut index, |tx| {
        write_repo_facts(tx, &binding(), &payload(), Some("W/\"abc\""), NOW).expect("200");
    });
    exercised += 1;
    assert_eq!(read(&index), (RemoteFactsState::Observed, Some(NOW)));

    // 304 — the server compared our validator and asserted the representation current (A14).
    in_tx(&mut index, |tx| {
        confirm_repo_facts(tx, &binding(), NOW + 60).expect("304");
    });
    exercised += 1;
    assert_eq!(read(&index), (RemoteFactsState::Observed, Some(NOW + 60)));

    // throttled — no call at all. The absence of a write IS the outcome.
    exercised += 1;
    assert_eq!(read(&index), (RemoteFactsState::Observed, Some(NOW + 60)));

    // skipped — likewise, and asserted separately because they are different decisions with the
    // same consequence, and a test that folded them would exercise one of them.
    exercised += 1;
    assert_eq!(read(&index), (RemoteFactsState::Observed, Some(NOW + 60)));

    // 403 — the access state, and nothing dated. The counts stay dated by the read that
    // produced them.
    in_tx(&mut index, |tx| {
        mark_not_permitted(tx, &binding()).expect("403");
    });
    exercised += 1;
    assert_eq!(
        read(&index),
        (RemoteFactsState::NotPermitted, Some(NOW + 60))
    );

    // 404 — the same answer for the same reason: a repository that does not exist and one this
    // token cannot see are indistinguishable, and neither observed a value.
    in_tx(&mut index, |tx| {
        mark_not_permitted(tx, &binding()).expect("404");
    });
    exercised += 1;
    assert_eq!(
        read(&index),
        (RemoteFactsState::NotPermitted, Some(NOW + 60))
    );

    eprintln!("acceptance_p2_remote_facts: exercised {exercised} outcome(s)");
    assert!(
        exercised >= 6,
        "a sequence test that silently exercised {exercised} of six proves nothing"
    );

    // The counts a 403 did not observe are still the ones the 200 wrote.
    let facts = remote_facts(index.conn(), ProjectId(1))
        .expect("read")
        .expect("facts");
    assert_eq!(facts.stars, Some(41));
    assert_eq!(facts.good_first_issues, Some(0));
}

/// **AC-P2-25-25-facts.** The facts row is keyed on the forge's stable id, so two projects on
/// one `remote_key` share one row and a rename leaves the row alone.
///
/// The no-unique-key clause over the migrated schema is **`AC-P2-25-25-schema`**, p2-22's, in
/// `core/tests/index_rebuild_0009.rs`, and is not repeated here.
#[test]
fn ac_p2_25_25_facts_are_keyed_on_the_stable_id_and_survive_a_rename() {
    let (_dir, mut index) = seeded();
    // §22.5's shape: two live projects carrying one `remote_key`, bound to one forge id.
    index
        .conn()
        .execute(
            "INSERT INTO project (id, name, seed_basename, remote_key, provider,
                                  provider_repo_id, remote_link_basis, last_touched_at,
                                  created_at, updated_at)
             VALUES (2, 'widget-too', 'widget-too', 'github.com/acme/widget', 'github', '909',
                     'provider_id', ?1, ?1, ?1)",
            rusqlite::params![NOW],
        )
        .expect("second project");
    in_tx(&mut index, |tx| {
        write_repo_facts(tx, &binding(), &payload(), Some("W/\"abc\""), NOW).expect("200");
    });

    let rows: i64 = index
        .conn()
        .query_row("SELECT COUNT(*) FROM remote_repo", [], |r| r.get(0))
        .expect("count");
    assert_eq!(
        rows, 1,
        "two projects on one forge repository share one facts row"
    );
    for project in [1_i64, 2] {
        let facts = remote_facts(index.conn(), ProjectId(project))
            .expect("read")
            .expect("facts");
        assert_eq!(
            facts.stars,
            Some(41),
            "project {project} lost the shared row"
        );
    }

    // A rename rewrites `remote_key` and nothing else. The stable id is what the facts hang on.
    index
        .conn()
        .execute(
            "UPDATE project SET remote_key = 'github.com/acme/widget-renamed' WHERE id = 1",
            [],
        )
        .expect("rename");
    let facts = remote_facts(index.conn(), ProjectId(1))
        .expect("read")
        .expect("facts");
    assert_eq!(facts.key, "github.com/acme/widget-renamed");
    assert_eq!(facts.stars, Some(41), "a rename cost the counts");
    assert_eq!(facts.observed_at, Some(NOW), "a rename moved the clock");

    // …and the stable id is on `project` and nowhere else: no `remote_*` table carries a
    // `remote_link_basis`, which is the identity half §22.9 keeps off a facts row.
    let mut stmt = index
        .conn()
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'remote_%'")
        .expect("prepare");
    let names: Vec<String> = stmt
        .query_map([], |r| r.get(0))
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("names");
    eprintln!(
        "acceptance_p2_remote_facts: schema scan read {} remote table(s)",
        names.len()
    );
    assert!(
        !names.is_empty(),
        "the schema scan read zero tables, so it proved nothing"
    );
    for table in &names {
        let mut columns = index
            .conn()
            .prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
            .expect("prepare");
        let cols: Vec<String> = columns
            .query_map([], |r| r.get(0))
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("cols");
        assert!(!cols.is_empty(), "{table} reported no columns");
        assert!(
            !cols.iter().any(|c| c == "remote_link_basis"),
            "{table} carries the identity basis, which lives on `project` alone"
        );
        // A15: an ETag is never rendered as a time, so §6 grants it no column of its own.
        assert!(
            !cols.iter().any(|c| c.ends_with("etag_observed_at")),
            "{table} carries a second clock for its validator"
        );
    }
}

/// **AC-P2-25-10-chain.** §5.2's chain becomes `manifest → remote → README → note → detected`,
/// and the whole chain is walked rather than one rung.
///
/// The DDL-and-enum mirror is **`AC-P2-25-10-ddl`**, p2-22's, in
/// `core/tests/index_rebuild_0009.rs`: it reads the CHECK text out of the migration and compares
/// it to the generated variant set. One value, one mirror test — a second copy here would be the
/// defect the mirror exists to catch, wearing the reviewer's coat.
#[test]
fn ac_p2_25_10_chain_resolves_manifest_then_remote_then_readme_then_note_then_detected() {
    use codotheca_core::derive::description::{describe, DescriptionSource};

    let manifest = Some("A tiny thing");
    let remote = Some("The forge's About line");
    let readme = Some("# Thing\nA library for one job.\n");
    let note = Some("my scratch pad");
    let lang = Some("Rust");
    let arch = Some("cli");

    // All five sources present.
    let all = describe(manifest, remote, readme, note, lang, arch);
    assert_eq!(all.source, Some(DescriptionSource::Manifest));
    assert_eq!(all.text.as_deref(), Some("A tiny thing"));

    // Remove the manifest — the forge's line, not the README's first sentence.
    let without_manifest = describe(None, remote, readme, note, lang, arch);
    assert_eq!(without_manifest.source, Some(DescriptionSource::Remote));
    assert_eq!(
        without_manifest.text.as_deref(),
        Some("The forge's About line")
    );

    let without_remote = describe(None, None, readme, note, lang, arch);
    assert_eq!(without_remote.source, Some(DescriptionSource::Readme));
    assert_eq!(
        without_remote.text.as_deref(),
        Some("A library for one job.")
    );

    let without_readme = describe(None, None, None, note, lang, arch);
    assert_eq!(without_readme.source, Some(DescriptionSource::Note));

    let detected = describe(None, None, None, None, lang, arch);
    assert_eq!(detected.source, Some(DescriptionSource::Detected));
    assert_eq!(detected.text.as_deref(), Some("Rust CLI"));

    // The case the rank exists for: a project that was never cloned has no manifest, no README
    // and no note, so the forge's line is the only description that exists.
    let never_cloned = describe(None, remote, None, None, None, None);
    assert_eq!(never_cloned.source, Some(DescriptionSource::Remote));
}

/// The chain above is a **pure function** called with literals, and that is the whole gap: it
/// proves `describe` ranks correctly and says nothing about whether the forge's line ever
/// reaches it.
///
/// `j6_content::persist`'s `LEFT JOIN remote_repo` is the sole supplier of rank 2 on any
/// production path, joined on §22.11's binding rather than on the non-unique `remote_key`. No
/// test put a `remote_repo.description` in a database and asserted a project's description
/// changed — so a join that silently matched nothing would leave the chain passing and rank 2
/// dead. R88's question, asked of the rung rather than of the ladder.
#[test]
fn the_forge_description_reaches_the_chain_through_j6s_own_join() {
    use codotheca_core::jobs::j6_content::{persist, ContentFacts};

    let (_dir, mut index) = seeded();
    in_tx(&mut index, |tx| {
        write_repo_facts(tx, &binding(), &payload(), None, NOW).expect("200");
    });

    // No manifest, no README, no note: rank 2 is the only rung with anything in it, so the
    // description that comes back is the forge's or the join did not fire.
    let facts = ContentFacts {
        manifest_description: None,
        readme_path: None,
        readme_excerpt: None,
        readme_seen: true,
    };
    in_tx(&mut index, |tx| {
        persist(tx, ProjectId(1), &facts, NOW).expect("j6 persists");
    });

    let (text, source): (Option<String>, Option<String>) = index
        .conn()
        .query_row(
            "SELECT description, description_source FROM project WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("read the project back");
    assert_eq!(
        text.as_deref(),
        Some("a widget"),
        "rank 2 is `remote_repo.description`, and it must arrive through the join"
    );
    assert_eq!(
        source.as_deref(),
        Some("remote"),
        "the source is stored, so a later reader can say where the sentence came from"
    );
}

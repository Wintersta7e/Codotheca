#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §45.3(a) and §45.6 step 6, through the handlers: uniqueness is read **after** the verifying
//! read, per remote; same-machine remotes contribute nothing; the composition names what did not
//! answer and never calls it unpushed; and no verification read runs that cannot matter.
//!
//! A remote that answers is the production verifier over the fixture transport. The
//! did-not-answer classes are `FixtureRemoteVerifier`'s (§45.14: the one seam a test may double).

mod support;

use std::collections::BTreeMap;
use std::path::Path;

use codotheca_core::analyser::remote::{classify_remote_url, DidNotAnswer, RemoteClass};
use codotheca_core::testing::FixtureRemoteVerifier;
use support::analyser_world::{Library, Verdict, NOW};

/// A commit only this repository holds, on a new branch `name`.
fn local_branch(lib: &Library, repo: &Path, name: &str) -> String {
    lib.git(repo, &["checkout", "-q", "-b", name]);
    let oid = lib.commit(repo, &format!("{name}.txt"), &format!("{name}\n"));
    lib.git(repo, &["checkout", "-q", "main"]);
    oid
}

/// `repo`'s `HEAD` commit.
fn head(lib: &Library, repo: &Path) -> String {
    lib.git(repo, &["rev-parse", "HEAD"]).trim().to_owned()
}

/// Every ref, reflog, `packed-refs`, `HEAD` and `FETCH_HEAD` under `.git`, by path, with its
/// bytes.
fn ref_state(git_dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    std::fs::read(&path).expect("read"),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(git_dir, &git_dir.join("refs"), &mut out);
    walk(git_dir, &git_dir.join("logs"), &mut out);
    for file in ["packed-refs", "HEAD", "FETCH_HEAD"] {
        if let Ok(bytes) = std::fs::read(git_dir.join(file)) {
            out.insert(file.to_owned(), bytes);
        }
    }
    out
}

/// **AC-P4-45-6 — uniqueness after the read, per remote.**
#[test]
fn ac_p4_45_6() {
    // A branch deleted upstream whose tracking ref survives: its commits are uncovered.
    {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        local_branch(&lib, &copy, "gone");
        lib.git(&copy, &["push", "-q", "origin", "gone"]);
        lib.git(&copy, &["fetch", "-q", "origin"]);
        lib.git(&lib.net.join("widget.git"), &["branch", "-D", "gone"]);
        assert!(copy.join(".git/refs/remotes/origin/gone").exists());
        let id = lib.register(&copy);
        let deleted = lib.preflight(id, &lib.verifier());
        eprintln!("a branch deleted upstream, its tracking ref kept: {deleted}");
        assert!(deleted.has("unpushed_commits"), "{deleted}");
    }

    // A second remote that never answers, whose tracking ref holds the commit: not counted.
    {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        let local = local_branch(&lib, &copy, "feature");
        lib.git(
            &copy,
            &[
                "remote",
                "add",
                "second",
                "https://forge.invalid/acme/second.git",
            ],
        );
        lib.git(
            &copy,
            &["update-ref", "refs/remotes/second/feature", &local],
        );
        let id = lib.register(&copy);
        let remotes = FixtureRemoteVerifier::new();
        remotes.answering("origin", &[head(&lib, &copy)]);
        remotes.silent("second", DidNotAnswer::Failed);
        let unanswered = lib.preflight(id, &remotes);
        eprintln!("a silent second remote holding the tracking ref: {unanswered}");
        assert_eq!(unanswered.blockers(), vec!["remote_unreachable".to_owned()]);
    }

    // Two network remotes, one offline and one covering every root: safe.
    {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        lib.git(
            &copy,
            &[
                "remote",
                "add",
                "second",
                "https://forge.invalid/acme/second.git",
            ],
        );
        let id = lib.register(&copy);
        let remotes = FixtureRemoteVerifier::new();
        remotes.answering("origin", &[head(&lib, &copy)]);
        remotes.silent("second", DidNotAnswer::Deadline);
        let covered = lib.preflight(id, &remotes);
        eprintln!("one offline remote beside one that covers every root: {covered}");
        assert_eq!(covered.disposition(), "safe", "{covered}");
    }

    byte_identical_across_the_analysis();
}

/// AC-P4-45-6's fourth clause: refs, every reflog, `packed-refs`, `HEAD` and `FETCH_HEAD` are
/// byte-identical across an analysis whose objects step ran — the origin holds a branch this
/// copy has never seen.
fn byte_identical_across_the_analysis() {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    let other = lib.base.join("other");
    lib.git(
        &lib.base,
        &[
            "clone",
            "-q",
            &lib.net.join("widget.git").to_string_lossy(),
            &other.to_string_lossy(),
        ],
    );
    let elsewhere = local_branch(&lib, &other, "elsewhere");
    lib.git(&other, &["push", "-q", "origin", "elsewhere"]);
    let id = lib.register(&copy);
    let present = |oid: &str| {
        lib.git_output(&copy, &["cat-file", "-e", oid])
            .status
            .success()
    };
    assert!(
        !present(&elsewhere),
        "the copy starts without the origin's new commit"
    );
    let before = ref_state(&copy.join(".git"));
    let verdict = lib.preflight(id, &lib.verifier());
    let after = ref_state(&copy.join(".git"));
    assert!(
        present(&elsewhere),
        "the objects step did not run, so this proves nothing"
    );
    eprintln!(
        "byte-identical across the analysis: {} ref files compared; {verdict}",
        before.len()
    );
    assert_eq!(verdict.disposition(), "safe", "{verdict}");
    assert_eq!(
        before, after,
        "the analysis wrote a ref, a reflog or FETCH_HEAD"
    );
    assert!(!before.is_empty());
}

/// One way to add a same-machine remote named `mirror`.
type Shape = fn(&Library, &Path);

/// A copy with a `mirror` added by `add_mirror` — beside a covering network origin when
/// `covered`, and otherwise as its only remote, with its one commit local — and its verdict.
fn with_mirror(add_mirror: Shape, covered: bool) -> Verdict {
    let lib = Library::new();
    let copy = if covered {
        lib.pushed_repo("widget")
    } else {
        lib.repo("widget")
    };
    add_mirror(&lib, &copy);
    let id = lib.register(&copy);
    lib.preflight(id, &lib.verifier())
}

/// **AC-P4-45-8 — same-machine remotes.** An `insteadOf` onto a local path, a `file://` remote,
/// a `127.0.0.1` https remote and a bare local path each contribute nothing: beside a covering
/// network remote the copy is `safe` with no `remote_is_local_mirror`; as the only remote, with a
/// local commit, `unpushed_commits` + `no_remote` + `remote_is_local_mirror`. A `git://` remote as
/// the only remote is `remote_unreachable`, never `no_remote`.
#[test]
fn ac_p4_45_8() {
    let shapes: [(&str, Shape); 4] = [
        ("an insteadOf onto a local path", |lib, repo| {
            let mirror = lib.base.join("m1.git");
            lib.git(
                repo,
                &[
                    "remote",
                    "add",
                    "mirror",
                    "https://forge.invalid/acme/m1.git",
                ],
            );
            lib.git(
                repo,
                &[
                    "config",
                    &format!("url.{}.insteadOf", mirror.to_string_lossy()),
                    "https://forge.invalid/acme/m1.git",
                ],
            );
        }),
        ("a file:// remote", |lib, repo| {
            let url = format!(
                "file://{}{}",
                if cfg!(windows) { "/" } else { "" },
                lib.base.join("m2.git").to_string_lossy().replace('\\', "/")
            );
            lib.git(repo, &["remote", "add", "mirror", &url]);
        }),
        ("a 127.0.0.1 https remote", |lib, repo| {
            lib.git(
                repo,
                &[
                    "remote",
                    "add",
                    "mirror",
                    "https://127.0.0.1/acme/widget.git",
                ],
            );
        }),
        ("a bare local path", |lib, repo| {
            lib.git(
                repo,
                &[
                    "remote",
                    "add",
                    "mirror",
                    &lib.base.join("m4.git").to_string_lossy(),
                ],
            );
        }),
    ];
    let mut exercised = 0;
    for (name, add_mirror) in shapes {
        let beside = with_mirror(add_mirror, true);
        let alone = with_mirror(add_mirror, false);
        eprintln!("same-machine, {name}: beside a covering remote {beside}; alone {alone}");
        assert_eq!(beside.disposition(), "safe", "{name}: {beside}");
        assert!(!beside.has("remote_is_local_mirror"), "{name}: {beside}");
        assert_eq!(
            alone.blockers(),
            vec![
                "no_remote".to_owned(),
                "remote_is_local_mirror".to_owned(),
                "unpushed_commits".to_owned()
            ],
            "{name}"
        );
        exercised += 1;
    }

    let not_admitted = {
        let lib = Library::new();
        let copy = lib.repo("widget");
        lib.git(
            &copy,
            &[
                "remote",
                "add",
                "origin",
                "git://forge.invalid/acme/widget.git",
            ],
        );
        let id = lib.register(&copy);
        lib.preflight(id, &lib.verifier())
    };
    eprintln!(
        "a git:// remote as the only remote: {not_admitted}; {exercised} same-machine shapes"
    );
    assert_eq!(
        not_admitted.blockers(),
        vec!["remote_unreachable".to_owned()]
    );
    assert_eq!(exercised, 4);
}

/// **AC-P4-45-11 — composition.** No remote yields `no_remote` and never `remote_unreachable`; with
/// the only remote offline, a copy whose commits were pushed yesterday yields
/// `remote_unreachable` and never `unpushed_commits`.
///
/// An empty repository with no remote and only junk is `safe`: nothing needs an elsewhere, and
/// junk is not precious (§45.4).
#[test]
fn ac_p4_45_11() {
    let none = {
        let lib = Library::new();
        let copy = lib.repo("widget");
        let id = lib.register(&copy);
        lib.preflight(id, &lib.verifier())
    };
    let yesterday = {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        let id = lib.register(&copy);
        let offline = FixtureRemoteVerifier::new();
        offline.silent("origin", DidNotAnswer::Failed);
        lib.preflight(id, &offline)
    };
    let junk_only = {
        let lib = Library::new();
        let empty = lib.root.join("widget");
        lib.git(
            &lib.base,
            &["init", "-q", "-b", "main", &empty.to_string_lossy()],
        );
        let junk = empty.join("node_modules").join("pkg");
        std::fs::create_dir_all(&junk).expect("junk");
        std::fs::write(junk.join("index.js"), b"module.exports = 1;\n").expect("junk file");
        let id = lib.register(&empty);
        lib.preflight(id, &lib.verifier())
    };
    eprintln!(
        "no remote: {none}; the only remote offline, pushed yesterday: {yesterday}; an empty \
         repository with only junk: {junk_only}"
    );
    assert_eq!(junk_only.disposition(), "safe", "{junk_only}");
    assert!(none.has("no_remote"), "{none}");
    assert!(!none.has("remote_unreachable"), "{none}");
    assert_eq!(yesterday.blockers(), vec!["remote_unreachable".to_owned()]);
}

/// **AC-P4-45-16's Uninstall half — no verification write that cannot matter.** A live session
/// and a stash are undischargeable for Uninstall: **zero** verifier calls, and every local
/// blocker still reported.
#[test]
fn no_verification_read_runs_when_a_local_blocker_is_undischargeable() {
    // A live launch session, beside an untracked file.
    let (live, live_calls) = {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        let id = lib.register(&copy);
        lib.index
            .lock()
            .expect("index")
            .conn()
            .execute(
                "INSERT INTO session (project_id, location_id, started_at)
                 SELECT project_id, id, 1 FROM location WHERE id = ?1",
                [id.0],
            )
            .expect("a live session");
        std::fs::write(copy.join("notes.txt"), b"unsaved\n").expect("untracked");
        let remotes = FixtureRemoteVerifier::new();
        remotes.answering("origin", &[head(&lib, &copy)]);
        (lib.preflight(id, &remotes), remotes.calls())
    };

    // An ignored file that is not junk.
    let (ignored, ignored_calls) = {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        std::fs::write(copy.join(".git/info/exclude"), b".env\n").expect("exclude");
        std::fs::write(copy.join(".env"), b"TOKEN=local\n").expect(".env");
        let id = lib.register(&copy);
        let remotes = FixtureRemoteVerifier::new();
        remotes.answering("origin", &[head(&lib, &copy)]);
        (lib.preflight(id, &remotes), remotes.calls())
    };

    // A stash.
    let (stashed, stash_calls) = {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        std::fs::write(copy.join("a.txt"), b"stashed\n").expect("edit");
        lib.git(&copy, &["stash", "push", "-q", "-m", "work"]);
        let id = lib.register(&copy);
        let remotes = FixtureRemoteVerifier::new();
        remotes.answering("origin", &[head(&lib, &copy)]);
        (lib.preflight(id, &remotes), remotes.calls())
    };

    eprintln!(
        "live session: {live}, {} verifier call(s); ignored .env: {ignored}, {} verifier \
         call(s); stash: {stashed}, {} verifier call(s)",
        live_calls.len(),
        ignored_calls.len(),
        stash_calls.len()
    );
    assert!(ignored.has("ignored_precious"), "{ignored}");
    assert!(ignored_calls.is_empty(), "{ignored_calls:?}");
    assert!(
        live.has("live_session") && live.has("untracked_precious"),
        "{live}"
    );
    assert!(stashed.has("stash_present"), "{stashed}");
    assert!(live_calls.is_empty(), "{live_calls:?}");
    assert!(stash_calls.is_empty(), "{stash_calls:?}");
}

/// **AC-P2-24-16.** 401, 403, 404, offline, the deadline and a refused transport are one claim
/// — *this was not established* — and **none of them is ever `safe`**, and none claims an instant.
///
/// A 404 against a private repository the caller cannot see is indistinguishable from one that
/// does not exist. Rendering it as *gone* is the specific mistake that turns this into a shredder.
#[test]
fn no_remote_failure_ever_yields_safe() {
    for why in [
        DidNotAnswer::Failed,
        DidNotAnswer::Deadline,
        DidNotAnswer::Unresolved,
        DidNotAnswer::TransportRefused,
        DidNotAnswer::Unparseable,
        DidNotAnswer::Unverifiable,
    ] {
        let lib = Library::new();
        let copy = lib.pushed_repo("widget");
        let id = lib.register(&copy);
        let remotes = FixtureRemoteVerifier::new();
        remotes.silent("origin", why);
        let verdict = lib.preflight(id, &remotes);
        assert_eq!(verdict.disposition(), "unknown", "{why:?}: {verdict}");
        assert!(
            verdict.0["remoteVerifiedAt"].is_null(),
            "{why:?}: {verdict}"
        );
    }
}

/// **AC-P2-24-16-mirror, renamed (§45.13).** A mirror on the same machine is never counted as a
/// backup — one disk failure takes both copies — and is **named only beside an uncovered root**:
/// alone over a copy with nothing to cover it says nothing, and beside an uncovered commit it
/// says why that remote does not count.
#[test]
fn a_local_mirror_is_named_only_beside_an_uncovered_root() {
    let mirror = |lib: &Library, repo: &Path| {
        let path = lib.base.join("mirror.git");
        lib.git(
            &lib.base,
            &[
                "init",
                "-q",
                "--bare",
                "-b",
                "main",
                &path.to_string_lossy(),
            ],
        );
        lib.git(repo, &["remote", "add", "mirror", &path.to_string_lossy()]);
    };

    // An empty repository: nothing to cover, so nothing to say.
    let quiet = {
        let lib = Library::new();
        let empty = lib.root.join("widget");
        lib.git(
            &lib.base,
            &["init", "-q", "-b", "main", &empty.to_string_lossy()],
        );
        mirror(&lib, &empty);
        let id = lib.register(&empty);
        lib.preflight(id, &lib.verifier())
    };

    // A commit only the mirror holds: named, and never a backup.
    let named = {
        let lib = Library::new();
        let copy = lib.repo("widget");
        mirror(&lib, &copy);
        lib.git(&copy, &["push", "-q", "mirror", "main"]);
        let id = lib.register(&copy);
        lib.preflight(id, &lib.verifier())
    };
    eprintln!("a mirror over nothing uncovered: {quiet}; beside an uncovered commit: {named}");
    assert!(!quiet.has("remote_is_local_mirror"), "{quiet}");
    assert!(named.has("remote_is_local_mirror"), "{named}");
    assert!(named.has("unpushed_commits"), "{named}");
    assert_ne!(named.disposition(), "safe");
}

/// The label is `VERIFIED <age>`, never `PUSHED` — so the instant has to be real: the call's own
/// clock, and only when a network remote answered.
#[test]
fn a_reached_remote_records_when_it_was_reached() {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    let id = lib.register(&copy);
    let verdict = lib.preflight(id, &lib.verifier());
    assert_eq!(verdict.disposition(), "safe", "{verdict}");
    assert_eq!(verdict.0["remoteVerifiedAt"].as_i64(), Some(NOW));
}

/// §45.3(a)'s same-machine class, however the path is spelled; https, ssh and scp are network.
#[test]
fn a_local_path_remote_is_recognised_however_it_is_spelled() {
    for local in [
        "file:///srv/mirrors/widget.git",
        "/srv/mirrors/widget.git",
        "../widget.git",
        "~/mirrors/widget.git",
        "D:\\Mirrors\\widget.git",
        "https://localhost/acme/widget.git",
        "ssh://git@127.0.0.1/acme/widget.git",
    ] {
        assert_eq!(
            classify_remote_url(local),
            RemoteClass::SameMachine,
            "{local} is on this machine"
        );
    }
    for remote in [
        "https://forge.example/owner/widget",
        "ssh://git@forge.example/owner/widget.git",
        "git@forge.example:owner/widget.git",
    ] {
        assert_eq!(
            classify_remote_url(remote),
            RemoteClass::Network,
            "{remote} is a real remote"
        );
    }
}

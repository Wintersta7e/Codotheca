#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §45.5's Lane-0 refusals, through the handlers: each yields its blocker, and each is
//! undischargeable for Uninstall.

mod support;

use support::analyser_world::{Library, Verdict};

/// A main checkout with a linked worktree elsewhere.
fn a_main_checkout_with_a_linked_worktree_elsewhere() -> Verdict {
    let lib = Library::new();
    let main = lib.pushed_repo("widget");
    lib.git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            &lib.base.join("elsewhere").to_string_lossy(),
        ],
    );
    let id = lib.register(&main);
    lib.preflight(id, &lib.verifier())
}

/// A linked worktree.
fn a_linked_worktree() -> Verdict {
    let lib = Library::new();
    let main = lib.pushed_repo("widget");
    let linked = lib.root.join("linked");
    lib.git(&main, &["worktree", "add", "-q", &linked.to_string_lossy()]);
    let id = lib.register(&linked);
    lib.preflight(id, &lib.verifier())
}

/// Another location's alternates pointing in.
fn alternates_pointing_in() -> Verdict {
    let lib = Library::new();
    let lender = lib.pushed_repo("widget");
    let borrower = lib.root.join("borrower");
    lib.git(
        &lib.base,
        &[
            "clone",
            "-q",
            "--shared",
            &lender.to_string_lossy(),
            &borrower.to_string_lossy(),
        ],
    );
    lib.register(&borrower);
    let id = lib.register(&lender);
    lib.preflight(id, &lib.verifier())
}

/// Another location's git dir inside this tree.
fn another_git_dir_inside() -> Verdict {
    let lib = Library::new();
    let host = lib.pushed_repo("widget");
    let outside = lib.root.join("outside");
    let inner = host.join("inner.git");
    lib.git(
        &lib.base,
        &[
            "init",
            "-q",
            "-b",
            "main",
            "--separate-git-dir",
            &inner.to_string_lossy(),
            &outside.to_string_lossy(),
        ],
    );
    let other = lib.register(&outside);
    lib.set_common_dir(other, &inner);
    // The git dir it holds is the host's to keep: ignored, so it is no untracked work of its own.
    std::fs::write(host.join(".git/info/exclude"), b"inner.git/\n").expect("exclude");
    let id = lib.register(&host);
    lib.preflight(id, &lib.verifier())
}

/// A `--separate-git-dir` location.
fn a_separate_git_dir_location() -> Verdict {
    let lib = Library::new();
    let separate = lib.root.join("separate");
    lib.git(
        &lib.base,
        &[
            "init",
            "-q",
            "-b",
            "main",
            "--separate-git-dir",
            &lib.base.join("separate.git").to_string_lossy(),
            &separate.to_string_lossy(),
        ],
    );
    lib.commit(&separate, "a.txt", "one\n");
    let id = lib.register(&separate);
    lib.preflight(id, &lib.verifier())
}

/// A path containing another location.
fn a_path_containing_another_location() -> Verdict {
    let lib = Library::new();
    let outer = lib.pushed_repo("widget");
    let nested = outer.join("nested");
    lib.git(
        &lib.base,
        &["init", "-q", "-b", "main", &nested.to_string_lossy()],
    );
    lib.register(&nested);
    let id = lib.register(&outer);
    lib.preflight(id, &lib.verifier())
}

/// A path containing a scan root.
fn a_path_containing_a_scan_root() -> Verdict {
    let lib = Library::new();
    let outer = lib.pushed_repo("widget");
    let root = outer.join("sub");
    std::fs::create_dir_all(&root).expect("root");
    lib.index
        .lock()
        .expect("index")
        .with_tx(|tx| {
            tx.execute(
                "INSERT INTO scan_root (kind, distro, path_bytes, path_key, path_display,
                                        added_by, added_at)
                 VALUES ('linux', '', ?1, ?1, ?2, 'user', 0)",
                rusqlite::params![root.to_string_lossy().as_bytes(), root.to_string_lossy()],
            )?;
            Ok(())
        })
        .expect("nested root");
    let id = lib.register(&outer);
    lib.preflight(id, &lib.verifier())
}

/// An interrupted cherry-pick.
fn an_interrupted_cherry_pick() -> Verdict {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    lib.git(&copy, &["checkout", "-q", "-b", "side"]);
    let pick = lib.commit(&copy, "a.txt", "side\n");
    lib.git(&copy, &["checkout", "-q", "main"]);
    lib.commit(&copy, "a.txt", "main\n");
    let conflicted = lib.git_output(&copy, &["cherry-pick", &pick]);
    assert!(
        !conflicted.status.success(),
        "the fixture's pick must conflict"
    );
    let id = lib.register(&copy);
    lib.preflight(id, &lib.verifier())
}

/// One refusal's fixture, built and pre-flighted.
type Case = fn() -> Verdict;

/// **AC-P4-45-12 — refusals** (Lane 0; the sealed-parcel folder is §46's, and the reftable
/// cherry-pick is the Windows-native gate's). Each yields its §45.5 blocker and is never `safe`.
#[test]
fn ac_p4_45_12() {
    let cases: [(&str, Case, &str); 8] = [
        (
            "a main checkout with a linked worktree elsewhere",
            a_main_checkout_with_a_linked_worktree_elsewhere,
            "linked_worktree",
        ),
        ("a linked worktree", a_linked_worktree, "linked_worktree"),
        (
            "another location's alternates pointing in",
            alternates_pointing_in,
            "borrowed_by_another_repository",
        ),
        (
            "another location's git dir inside this tree",
            another_git_dir_inside,
            "borrowed_by_another_repository",
        ),
        (
            "a --separate-git-dir location",
            a_separate_git_dir_location,
            "refused_path",
        ),
        (
            "a path containing another location",
            a_path_containing_another_location,
            "refused_path",
        ),
        (
            "a path containing a scan root",
            a_path_containing_a_scan_root,
            "refused_path",
        ),
        (
            "an interrupted cherry-pick",
            an_interrupted_cherry_pick,
            "interrupted_operation",
        ),
    ];
    let mut refused = 0;
    for (name, build, expected) in cases {
        let verdict = build();
        eprintln!("refusal, {name}: {verdict}");
        assert!(verdict.has(expected), "{name}: {verdict}");
        assert_ne!(verdict.disposition(), "safe", "{name}: {verdict}");
        refused += 1;
    }
    eprintln!("refusals: {refused} of 8 yielded their blocker");
    assert_eq!(refused, 8);
}

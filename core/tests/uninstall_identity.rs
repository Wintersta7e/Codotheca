#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
//! §45.6 step 1 through the handlers: **the directory is compared with its row, never with
//! itself.** Phase 2 built the uninstall warrant's expected identity from the directory it was
//! about to check (§37.8), so a working copy replaced by another repository matched itself and
//! went to the trash.
//!
//! Every copy here is under the hostile read profile, read by the real read backend, and its
//! origin is read by the production verifier over the fixture transport — so a copy that passes
//! step 1 is `safe`, which is what makes a refusal here a refusal of identity and nothing else.

mod support;

use std::collections::BTreeMap;
use std::path::Path;

use codotheca_core::testing::CountingTrash;
use support::analyser_world::Library;

/// Every file under `dir`, by relative path, with its bytes.
fn tree_bytes(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("read_dir").flatten() {
            let path = entry.path();
            let kind = entry.file_type().expect("file type");
            if kind.is_dir() {
                walk(base, &path, out);
            } else if kind.is_file() {
                let rel = path.strip_prefix(base).expect("under base");
                out.insert(
                    rel.to_string_lossy().into_owned(),
                    std::fs::read(&path).expect("read"),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// **AC-P4-45-2.** The directory is replaced by a repository of a different lineage between the
/// pre-flight and the act. The act refuses at step 1, the trash is sent **nothing**, and the
/// replacement is byte-identical afterwards. Put back, the original goes through — so the
/// refusal was identity's and not a verdict that was never going to be safe.
#[test]
fn a_replaced_directory_is_refused_through_the_handler() {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    let id = lib.register(&copy);
    let verifier = lib.verifier();
    let before = lib.preflight(id, &verifier);
    eprintln!("the original: {before}");
    assert_eq!(
        before.disposition(),
        "safe",
        "the fixture must be removable for a refusal to mean anything: {before}"
    );

    let aside = lib.base.join("original-aside");
    std::fs::rename(&copy, &aside).expect("move the original aside");
    lib.pushed_repo_at(&copy, "replacement");
    let replacement = tree_bytes(&copy);

    let trash = CountingTrash::new();
    let outcome = lib.uninstall(id, &verifier, &trash);
    eprintln!(
        "the act on the replacement: {outcome:?}; {} send(s)",
        trash.sends()
    );
    assert_eq!(trash.sends(), 0, "the trash was sent a replaced directory");
    let refused = outcome.expect_err("a replaced directory is refused");
    assert!(refused.contains("RefusedPath"), "{refused}");
    assert_eq!(
        tree_bytes(&copy),
        replacement,
        "the replacement changed under a refused act"
    );

    let verdict = lib.preflight(id, &verifier);
    eprintln!("the replacement's pre-flight: {verdict}");
    assert_eq!(verdict.disposition(), "blocked");
    assert_eq!(verdict.blockers(), vec!["refused_path".to_owned()]);

    std::fs::rename(&copy, lib.base.join("replacement-aside")).expect("move it aside");
    std::fs::rename(&aside, &copy).expect("put the original back");
    let removed = lib.uninstall(id, &verifier, &trash);
    eprintln!(
        "the act on the original: {}; {} send(s)",
        removed
            .as_ref()
            .map_or_else(Clone::clone, |_| "removed".to_owned()),
        trash.sends()
    );
    assert!(removed.is_ok(), "{removed:?}");
    assert_eq!(trash.sends(), 1);
}

/// The directory unchanged and **the row's `lineage_key` changed instead**: the same refusal,
/// because the comparison is with the row.
#[test]
fn a_changed_row_lineage_is_refused_through_the_handler() {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    let id = lib.register(&copy);
    {
        let guard = lib.index.lock().expect("index");
        guard
            .conn()
            .execute("UPDATE project SET lineage_key = ?1", [&"f".repeat(64)])
            .expect("lineage changed");
    }
    let unchanged = tree_bytes(&copy);
    let verifier = lib.verifier();
    assert_eq!(
        lib.preflight(id, &verifier).blockers(),
        vec!["refused_path".to_owned()]
    );
    let trash = CountingTrash::new();
    let refused = lib.uninstall(id, &verifier, &trash).expect_err("refused");
    eprintln!(
        "a changed row lineage: {refused}; {} send(s)",
        trash.sends()
    );
    assert_eq!(trash.sends(), 0);
    assert_eq!(tree_bytes(&copy), unchanged);
}

/// **Two NULLs match only when the live root set is empty and not shallow.** An empty
/// repository matches a NULL row and no other; a repository with commits never matches a NULL
/// row.
#[test]
fn an_empty_live_root_set_matches_only_a_null_row() {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    let id = lib.register(&copy);
    let verifier = lib.verifier();
    let set_row = |lineage: Option<&str>| {
        let guard = lib.index.lock().expect("index");
        guard
            .conn()
            .execute("UPDATE project SET lineage_key = ?1", [lineage])
            .expect("row lineage");
    };

    // Commits against a NULL row.
    set_row(None);
    let with_commits = lib.preflight(id, &verifier);

    // No commits against a NULL row, then against a lineage.
    std::fs::rename(&copy, lib.base.join("aside")).expect("aside");
    lib.git(
        &lib.base,
        &["init", "-q", "-b", "main", &copy.to_string_lossy()],
    );
    let empty_null = lib.preflight(id, &verifier);
    set_row(Some(&"e".repeat(64)));
    let empty_keyed = lib.preflight(id, &verifier);
    eprintln!(
        "commits vs NULL {with_commits}; empty vs NULL {empty_null}; empty vs a lineage \
         {empty_keyed}"
    );
    assert_eq!(with_commits.blockers(), vec!["refused_path".to_owned()]);
    assert!(!empty_null.has("refused_path"), "{empty_null}");
    assert!(!empty_null.has("refs_unreadable"), "{empty_null}");
    assert_eq!(empty_keyed.blockers(), vec!["refused_path".to_owned()]);
}

/// **A live shallow copy yields `shallow_clone`, never `refused_path`** (D-2): it has no
/// lineage by construction, so step 1 cannot match it and must not call it another repository.
#[test]
fn a_live_shallow_copy_yields_shallow_clone() {
    let lib = Library::new();
    let copy = lib.pushed_repo("widget");
    let id = lib.register(&copy);
    lib.commit(&copy, "b.txt", "two\n");
    lib.git(&copy, &["push", "-q", "origin", "main"]);
    std::fs::rename(&copy, lib.base.join("aside")).expect("aside");
    let url = format!(
        "file://{}{}",
        if cfg!(windows) { "/" } else { "" },
        lib.net
            .join("widget.git")
            .to_string_lossy()
            .replace('\\', "/")
    );
    lib.git(
        &lib.base,
        &["clone", "-q", "--depth", "1", &url, &copy.to_string_lossy()],
    );
    let verifier = lib.verifier();
    let verdict = lib.preflight(id, &verifier);
    eprintln!("a live shallow copy: {verdict}");
    assert!(verdict.has("shallow_clone"), "{verdict}");
    assert!(!verdict.has("refused_path"), "{verdict}");
    let trash = CountingTrash::new();
    assert!(lib.uninstall(id, &verifier, &trash).is_err());
    assert_eq!(trash.sends(), 0);
}

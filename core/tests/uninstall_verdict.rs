#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §24.8's fold, and the seal that never leaves the core.

use codotheca_core::analyser::verdict::{fold_disposition, is_unknown_blocker, VerdictSeal};
use codotheca_core::protocol::{UninstallBlocker, UninstallDisposition};

#[test]
fn no_blockers_is_safe() {
    assert_eq!(fold_disposition(&[]), UninstallDisposition::Safe);
}

#[test]
fn only_unknowns_is_unknown() {
    assert_eq!(
        fold_disposition(&[UninstallBlocker::RemoteUnreachable]),
        UninstallDisposition::Unknown
    );
}

/// `blocked` wins, **and both blockers are still listed** — the fold decides a disposition, it
/// never edits the list.
#[test]
fn a_known_bad_blocker_wins_and_hides_nothing() {
    let blockers = [
        UninstallBlocker::RemoteUnreachable,
        UninstallBlocker::UnpushedCommits,
    ];
    assert_eq!(fold_disposition(&blockers), UninstallDisposition::Blocked);
    assert_eq!(
        blockers.len(),
        2,
        "the caller's list is untouched: §24.8 requires every blocker to be reported"
    );
}

/// The two classes **partition** every blocker the schema declares: no overlap, no omission. A
/// new variant fails to compile inside `is_unknown`'s exhaustive match rather than falling into a
/// default, and the set iterated here is the generated `UninstallBlocker::ALL`, never a hand
/// list — a count written into this file is one value stated twice (§45.9, AC-P4-45-1).
#[test]
fn the_two_classes_partition_every_blocker_the_schema_declares() {
    let all = UninstallBlocker::ALL;
    assert!(
        !all.is_empty(),
        "the schema declares zero blockers, so the partition asserts nothing"
    );
    for blocker in all {
        let class = if is_unknown_blocker(blocker) {
            "unknown"
        } else {
            "known-bad"
        };
        eprintln!("class {blocker:?} {class}");
    }
    let unknowns: Vec<_> = all.iter().filter(|b| is_unknown_blocker(**b)).collect();
    let known: Vec<_> = all.iter().filter(|b| !is_unknown_blocker(**b)).collect();
    eprintln!(
        "known-bad {} / unknown {} / total {}",
        known.len(),
        unknowns.len(),
        all.len()
    );
    assert!(
        !unknowns.is_empty() && !known.is_empty(),
        "both classes must be inhabited"
    );
    assert_eq!(unknowns.len() + known.len(), all.len());
    // §45.9: it appears only beside another blocker, and as a known-bad member it would turn
    // *the network remote was offline* into `blocked`.
    assert!(
        is_unknown_blocker(UninstallBlocker::RemoteIsLocalMirror),
        "remote_is_local_mirror moved to the unknown class"
    );

    // Every blocker folds to a disposition on its own, and an unknown never folds to `blocked`
    // by itself.
    for blocker in all {
        let folded = fold_disposition(&[blocker]);
        assert_ne!(
            folded,
            UninstallDisposition::Safe,
            "{blocker:?} blocks something"
        );
        if is_unknown_blocker(blocker) {
            assert_eq!(folded, UninstallDisposition::Unknown, "{blocker:?}");
        } else {
            assert_eq!(folded, UninstallDisposition::Blocked, "{blocker:?}");
        }
    }
}

#[test]
fn a_seal_is_over_the_set_and_not_its_discovery_order() {
    let a = VerdictSeal::of(
        &[
            UninstallBlocker::UnpushedCommits,
            UninstallBlocker::StashPresent,
        ],
        UninstallDisposition::Blocked,
    );
    let b = VerdictSeal::of(
        &[
            UninstallBlocker::StashPresent,
            UninstallBlocker::UnpushedCommits,
        ],
        UninstallDisposition::Blocked,
    );
    assert_eq!(
        a, b,
        "the order blockers were found in is not an input to the decision"
    );
    let c = VerdictSeal::of(
        &[UninstallBlocker::StashPresent],
        UninstallDisposition::Blocked,
    );
    assert_ne!(a, c, "a different blocker set must seal differently");
}

/// §24.8: the seal is in-core only. **A scanning check, printing its file count**, because a
/// token that reached the wire would be a capability the renderer could hold and replay.
#[test]
fn the_seal_appears_in_no_generated_file_and_in_no_serialised_struct() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let generated = root.join("src").join("protocol.rs");
    let generated_text = std::fs::read_to_string(&generated).expect("the generated protocol");
    assert!(
        !generated_text.contains("VerdictSeal"),
        "the seal reached the generated protocol, which means it reached the wire"
    );

    let schema = root
        .parent()
        .expect("repo root")
        .join("protocol")
        .join("schema")
        .join("protocol.json");
    let schema_text = std::fs::read_to_string(&schema).expect("the schema");
    assert!(
        !schema_text.contains("VerdictSeal"),
        "the seal is not a schema type"
    );

    // And it carries no serde derive of its own.
    let mut scanned = 0_usize;
    let mut sealed_with_serde = Vec::new();
    for entry in walk(&root.join("src")) {
        let Ok(text) = std::fs::read_to_string(&entry) else {
            continue;
        };
        scanned += 1;
        if !text.contains("VerdictSeal") {
            continue;
        }
        for (index, line) in text.lines().enumerate() {
            if line.contains("VerdictSeal") && line.contains("struct") {
                // The derive sits on the line or two above the struct.
                let start = index.saturating_sub(2);
                let window = text.lines().skip(start).take(index - start + 1);
                if window
                    .clone()
                    .any(|l| l.contains("Serialize") || l.contains("Deserialize"))
                {
                    sealed_with_serde.push(entry.display().to_string());
                }
            }
        }
    }
    eprintln!("uninstall_verdict: the seal scan read {scanned} source file(s)");
    assert!(
        scanned > 100,
        "the scan read {scanned} files — wrong directory"
    );
    assert!(
        sealed_with_serde.is_empty(),
        "VerdictSeal is serialised in {sealed_with_serde:?}"
    );
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

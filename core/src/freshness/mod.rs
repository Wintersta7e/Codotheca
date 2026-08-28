//! §6 — what is cacheable and what is never cacheable.
//!
//! **This module owns the ref-state digest, and it is the only one.** Plan 05 shipped a second
//! digest of its own in `core::git::refstate`, which meant J1 would write one value into
//! `location.refstate_basis` while the freshness gate computed another and compared the two:
//! they can never be equal, so every cacheable job would re-run forever. `ref_fingerprint` now
//! collects [`BasisInputs`] from a `RepoHandle` and calls [`compute_basis`] — one digest, one
//! owner, and `core/tests/freshness_basis.rs` reads both sides rather than restating either.

pub mod watch;

use std::io::Read as _;
use std::path::Path;

use sha2::{Digest as _, Sha256};

use crate::git::RefFingerprint;

/// Every input §6's ref-state basis is computed over.
///
/// A digest, and a digest has no age (§6). Anything a surface renders as a *time* needs a
/// column of its own; exactly one does, and it is `location.fetch_head_at`.
///
/// ~~`pub struct RefStateBasis(pub String);`~~ **[superseded by R5]** — plan 05 owns the basis
/// type as `RefFingerprint(String)`, lowercase hex, matching `refstate_basis TEXT`. Two names
/// for one digest is exactly the drift the rulings exist to remove.
// Six bools, and `struct_excessive_bools` wants a state machine. It is wrong here: each is an
// independent predicate about one file on disk, and they co-occur freely — a shallow clone can
// be mid-rebase. Two-variant enums would restate `bool` six times and the digest would feed the
// same six bits.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BasisInputs {
    /// Raw bytes of `.git/HEAD`. This is the symbolic branch target.
    pub head_content: Vec<u8>,
    /// `packed-refs` mtime, in the same unit as `loose_refs`.
    pub packed_refs_mtime: Option<i64>,
    /// Every file under `refs/**`, as (ref path with `/` separators, mtime **in nanoseconds**).
    ///
    /// Nanoseconds, not seconds: two ref updates inside one second are ordinary during a fetch,
    /// and a second-resolution basis would call the repository unchanged across both.
    pub loose_refs: Vec<(String, i64)>,
    /// `FETCH_HEAD` mtime.
    pub fetch_head_mtime: Option<i64>,
    /// `config` mtime — an upstream can be configured without any ref moving.
    pub config_mtime: Option<i64>,
    /// A `shallow` file exists, so history is bounded.
    pub shallow_present: bool,
    /// `MERGE_HEAD`.
    pub merge_head_present: bool,
    /// `REBASE_HEAD`, `rebase-merge/` or `rebase-apply/`.
    pub rebase_head_present: bool,
    /// `CHERRY_PICK_HEAD`.
    pub cherry_pick_head_present: bool,
    /// `REVERT_HEAD`.
    pub revert_head_present: bool,
    /// `BISECT_LOG`.
    pub bisect_log_present: bool,
    /// `logs/refs/stash` mtime. `RefState.stash_count` is read from it, so a stash that does not
    /// move the basis leaves the count stale.
    pub stash_reflog_mtime: Option<i64>,
}

fn feed_opt(h: &mut Sha256, label: &str, v: Option<i64>) {
    h.update(label.as_bytes());
    h.update([0]);
    match v {
        Some(n) => h.update(n.to_le_bytes()),
        None => h.update([0xff]),
    }
    h.update([0]);
}

/// §6's basis. Pure: no filesystem, no clock, no process.
#[must_use]
pub fn compute_basis(inputs: &BasisInputs) -> RefFingerprint {
    RefFingerprint::from_digest(basis_hasher(inputs).finalize())
}

/// The digest state before it is finalised, so `observation_fingerprint` can extend it with the
/// index stat rather than declaring a second digest over the same inputs.
pub(crate) fn basis_hasher(inputs: &BasisInputs) -> Sha256 {
    let mut h = Sha256::new();
    h.update(b"codotheca/refstate/1\0");
    h.update(b"head\0");
    h.update(&inputs.head_content);
    h.update([0]);

    feed_opt(&mut h, "packed_refs", inputs.packed_refs_mtime);
    feed_opt(&mut h, "fetch_head", inputs.fetch_head_mtime);
    feed_opt(&mut h, "config", inputs.config_mtime);
    feed_opt(&mut h, "stash", inputs.stash_reflog_mtime);

    let mut refs = inputs.loose_refs.clone();
    refs.sort();
    h.update(b"refs\0");
    h.update(u64::try_from(refs.len()).unwrap_or(u64::MAX).to_le_bytes());
    for (name, mtime) in &refs {
        h.update(name.as_bytes());
        h.update([0]);
        h.update(mtime.to_le_bytes());
        h.update([0]);
    }

    h.update(b"flags\0");
    h.update([
        u8::from(inputs.shallow_present),
        u8::from(inputs.merge_head_present),
        u8::from(inputs.rebase_head_present),
        u8::from(inputs.cherry_pick_head_present),
        u8::from(inputs.revert_head_present),
        u8::from(inputs.bisect_log_present),
    ]);
    h
}

/// mtime in nanoseconds since the epoch. See [`BasisInputs::loose_refs`] for why not seconds.
fn mtime_nanos(p: &Path) -> Option<i64> {
    let meta = std::fs::metadata(p).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(dur.as_nanos()).ok()
}

/// Collect the inputs for a repository whose refs and worktree share one directory.
///
/// §3.3: ref state is direct file reads, ~1–5 ms. Nothing here spawns a process.
///
/// A linked worktree keeps its refs in the *common* dir, so use
/// [`collect_basis_inputs_split`] wherever the two can differ — which is everywhere a
/// `RepoHandle` is in hand.
pub fn collect_basis_inputs(git_dir: &Path) -> std::io::Result<BasisInputs> {
    collect_basis_inputs_split(git_dir, git_dir)
}

/// Collect the inputs, reading per-checkout state from `git_dir` and shared ref state from
/// `common_dir`. A linked worktree has its own `HEAD` and its own operation markers, and shares
/// everything else.
pub fn collect_basis_inputs_split(
    git_dir: &Path,
    common_dir: &Path,
) -> std::io::Result<BasisInputs> {
    let mut head_content = Vec::new();
    if let Ok(f) = std::fs::File::open(git_dir.join("HEAD")) {
        // A HEAD longer than a kilobyte is not a HEAD; cap the read rather than trust it.
        f.take(1024).read_to_end(&mut head_content)?;
    }

    let mut loose_refs = Vec::new();
    let mut stack = vec![(common_dir.join("refs"), 0_u32)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 16 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push((path, depth.saturating_add(1)));
                continue;
            }
            let Ok(rel) = path.strip_prefix(common_dir) else {
                continue;
            };
            let name = rel
                .components()
                .filter_map(|c| c.as_os_str().to_str())
                .collect::<Vec<_>>()
                .join("/");
            loose_refs.push((name, mtime_nanos(&path).unwrap_or(0)));
        }
    }

    Ok(BasisInputs {
        head_content,
        packed_refs_mtime: mtime_nanos(&common_dir.join("packed-refs")),
        loose_refs,
        fetch_head_mtime: mtime_nanos(&common_dir.join("FETCH_HEAD")),
        config_mtime: mtime_nanos(&common_dir.join("config")),
        shallow_present: common_dir.join("shallow").exists(),
        merge_head_present: git_dir.join("MERGE_HEAD").exists(),
        rebase_head_present: git_dir.join("REBASE_HEAD").exists()
            || git_dir.join("rebase-merge").exists()
            || git_dir.join("rebase-apply").exists(),
        cherry_pick_head_present: git_dir.join("CHERRY_PICK_HEAD").exists(),
        revert_head_present: git_dir.join("REVERT_HEAD").exists(),
        bisect_log_present: git_dir.join("BISECT_LOG").exists(),
        stash_reflog_mtime: mtime_nanos(&common_dir.join("logs").join("refs").join("stash")),
    })
}

/// §1.1 / §6: the reachable ref set at observation time, plus shallow-boundary state.
/// Invalidates on deepening, rewrite and a new orphan branch — history caching is not permanent.
#[must_use]
pub fn history_basis(ref_vector: &[(String, String)], shallow_boundary: Option<&str>) -> String {
    let mut refs = ref_vector.to_vec();
    refs.sort();
    let mut h = Sha256::new();
    h.update(b"codotheca/history/1\0");
    for (name, oid) in &refs {
        h.update(name.as_bytes());
        h.update([0]);
        h.update(oid.as_bytes());
        h.update([0]);
    }
    h.update(b"shallow\0");
    h.update(shallow_boundary.unwrap_or("").as_bytes());
    format!("{:x}", h.finalize())
}

use crate::jobs::JobKind;

/// What the database remembers about an observation. `basis` is `None` for worktree state, and
/// there is no variant of this type that could carry one: §6's model is that no fingerprint for
/// it exists.
///
/// **Named `StoredObservation`, not the plan's `Observation`.** Plan 05 already ships
/// `core::git::Observation<T>` — a *fresh read* plus the basis it was true against, with a
/// mandatory basis and a value. This is the other side: the row the freshness gate compares
/// against. Two different shapes, so neither collapses into the other, and neither should wear
/// the same name in one crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObservation {
    /// The basis the observation was taken against, `None` for a worktree reading.
    pub basis: Option<RefFingerprint>,
    /// When the app looked, epoch seconds — never when the user stopped.
    pub observed_at: i64,
}

/// §6's decision: run, or use what is stored.
#[derive(Debug)]
pub struct FreshnessGate;

impl FreshnessGate {
    /// Whether a job must run.
    ///
    /// The only way to answer "no" is a cacheable job whose stored basis equals the current one.
    /// Worktree state answers "yes" unconditionally, and the `is_cacheable` check is what makes
    /// that a property of the job kind rather than of a caller remembering to special-case it.
    #[must_use]
    pub fn needs_run(
        kind: JobKind,
        stored: Option<&StoredObservation>,
        current: Option<&RefFingerprint>,
    ) -> bool {
        if !kind.is_cacheable() {
            return true;
        }
        let (Some(stored), Some(current)) = (stored, current) else {
            return true;
        };
        stored.basis.as_ref() != Some(current)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::jobs::JobKind;

    fn base() -> BasisInputs {
        BasisInputs {
            head_content: b"ref: refs/heads/main\n".to_vec(),
            packed_refs_mtime: Some(1_000),
            loose_refs: vec![
                ("refs/heads/main".to_owned(), 2_000),
                ("refs/remotes/upstream/main".to_owned(), 2_000),
            ],
            fetch_head_mtime: Some(3_000),
            config_mtime: Some(500),
            shallow_present: false,
            merge_head_present: false,
            rebase_head_present: false,
            cherry_pick_head_present: false,
            revert_head_present: false,
            bisect_log_present: false,
            stash_reflog_mtime: None,
        }
    }

    #[test]
    fn the_basis_is_stable_and_order_independent() {
        let a = compute_basis(&base());
        let mut shuffled = base();
        shuffled.loose_refs.reverse();
        assert_eq!(a, compute_basis(&shuffled));
        assert_eq!(a.as_str().len(), 64);
    }

    #[test]
    fn a_fetch_moves_the_basis_without_touching_head_or_packed_refs() {
        // §6: ref-state fingerprints must cover loose refs and the symbolic branch target,
        // not just HEAD and packed-refs — a fetch changes ahead/behind without touching either.
        //
        // The plan's version of this test also moved `fetch_head_mtime`, so it passed with the
        // whole loose-ref block deleted from the digest — a bar written past the defect it was
        // written for. Only the remote-tracking ref moves here, so nothing else can be what
        // moved the result.
        let before = compute_basis(&base());
        let mut after = base();
        after.loose_refs = vec![
            ("refs/heads/main".to_owned(), 2_000),
            ("refs/remotes/upstream/main".to_owned(), 2_400),
        ];
        assert_eq!(base().head_content, after.head_content);
        assert_eq!(base().packed_refs_mtime, after.packed_refs_mtime);
        assert_eq!(base().fetch_head_mtime, after.fetch_head_mtime);
        assert_ne!(before, compute_basis(&after));
    }

    /// `FETCH_HEAD` moving on its own is also a change: a fetch that brought nothing new still
    /// re-dates the last-fetch clock §5.1 reads.
    #[test]
    fn fetch_head_alone_moves_the_basis() {
        let before = compute_basis(&base());
        let mut after = base();
        after.fetch_head_mtime = Some(3_400);
        assert_ne!(before, compute_basis(&after));
    }

    #[test]
    fn switching_the_symbolic_target_moves_the_basis() {
        let before = compute_basis(&base());
        let mut after = base();
        after.head_content = b"ref: refs/heads/release\n".to_vec();
        assert_ne!(before, compute_basis(&after));
    }

    #[test]
    fn a_new_loose_ref_moves_the_basis_even_at_the_same_mtime() {
        let before = compute_basis(&base());
        let mut after = base();
        after
            .loose_refs
            .push(("refs/heads/topic".to_owned(), 2_000));
        assert_ne!(before, compute_basis(&after));
    }

    #[test]
    fn operation_markers_and_shallow_are_in_the_basis() {
        let before = compute_basis(&base());
        let mut rebasing = base();
        rebasing.rebase_head_present = true;
        assert_ne!(before, compute_basis(&rebasing));
        let mut deepened = base();
        deepened.shallow_present = true;
        assert_ne!(before, compute_basis(&deepened));
    }

    /// Plan 05's digest covered three markers and the stash reflog that plan 09's `BasisInputs`
    /// did not. Unifying the two must not have quietly narrowed what invalidates the cache:
    /// a stash that does not move the basis leaves `stash_count` stale forever.
    #[test]
    fn the_remaining_markers_and_the_stash_are_in_the_basis_too() {
        let before = compute_basis(&base());
        for mutate in [
            (|b: &mut BasisInputs| b.cherry_pick_head_present = true) as fn(&mut BasisInputs),
            |b: &mut BasisInputs| b.revert_head_present = true,
            |b: &mut BasisInputs| b.bisect_log_present = true,
            |b: &mut BasisInputs| b.stash_reflog_mtime = Some(7),
        ] {
            let mut after = base();
            mutate(&mut after);
            assert_ne!(before, compute_basis(&after));
        }
    }

    #[test]
    fn a_matching_basis_skips_a_cacheable_job() {
        let basis = compute_basis(&base());
        let stored = StoredObservation {
            basis: Some(basis.clone()),
            observed_at: 10,
        };
        assert!(!FreshnessGate::needs_run(
            JobKind::J1Refstate,
            Some(&stored),
            Some(&basis)
        ));
    }

    #[test]
    fn a_moved_basis_reruns_a_cacheable_job() {
        let stored = StoredObservation {
            basis: Some(compute_basis(&base())),
            observed_at: 10,
        };
        let mut moved = base();
        moved.loose_refs.push(("refs/heads/topic".to_owned(), 9));
        assert!(FreshnessGate::needs_run(
            JobKind::J1Refstate,
            Some(&stored),
            Some(&compute_basis(&moved))
        ));
    }

    #[test]
    fn j2_reruns_even_when_every_fingerprint_is_identical() {
        // §6, the whole finding: v1 keyed the status cache on .git/index mtime + HEAD, and
        // editing a tracked file changes neither. An actively-worked repository would have
        // been cached as clean indefinitely and is:dirty would have silently lied.
        let basis = compute_basis(&base());
        let stored = StoredObservation {
            basis: Some(basis.clone()),
            observed_at: 10,
        };
        assert!(FreshnessGate::needs_run(
            JobKind::J2Status,
            Some(&stored),
            Some(&basis)
        ));
    }

    #[test]
    fn a_worktree_observation_carries_no_basis() {
        let stored = StoredObservation {
            basis: None,
            observed_at: 10,
        };
        assert!(FreshnessGate::needs_run(
            JobKind::J2Status,
            Some(&stored),
            None
        ));
    }

    /// A cacheable job that has never run has no stored observation, and "never observed" is
    /// not "unchanged".
    #[test]
    fn a_never_observed_cacheable_job_runs() {
        let basis = compute_basis(&base());
        assert!(FreshnessGate::needs_run(
            JobKind::J1Refstate,
            None,
            Some(&basis)
        ));
    }

    #[test]
    fn history_caching_is_not_permanent() {
        // §6: v1 said "never invalidate", which is wrong after a deepen, a rewrite or a new
        // orphan branch. The basis is the reachable ref set plus the shallow boundary.
        let full = history_basis(&[("refs/heads/main".to_owned(), "aaa".to_owned())], None);
        let deepened = history_basis(
            &[("refs/heads/main".to_owned(), "aaa".to_owned())],
            Some("bbb"),
        );
        assert_ne!(full, deepened);
        let orphan = history_basis(
            &[
                ("refs/heads/main".to_owned(), "aaa".to_owned()),
                ("refs/heads/orphan".to_owned(), "ccc".to_owned()),
            ],
            None,
        );
        assert_ne!(full, orphan);
    }
}

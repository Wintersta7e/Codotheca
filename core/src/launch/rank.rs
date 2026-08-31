//! §4bis.2's five-step evidence ranking for the **global** default editor.
//!
//! First hit wins, a step with no winner is skipped, and a step that ties is not a hit — it
//! falls through, exactly as a resolution tier does. §4bis.2a confines this to tier 4: nothing
//! here may consult a language.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::index::path::native_platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorEvidence {
    /// Index into the caller's candidate list; the ranker never reorders that list.
    pub index: usize,
    pub stem: String,
    pub recent_paths: Vec<PathBuf>,
    pub state_mtime: Option<i64>,
    pub installed_at: Option<i64>,
}

#[derive(Debug)]
pub struct RankInputs<'a> {
    pub editors: &'a [EditorEvidence],
    /// `path_key` of every location the scan has indexed.
    pub known_locations: &'a BTreeSet<Vec<u8>>,
    pub editor_env_stem: Option<&'a str>,
    pub folder_handler_stem: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankStep {
    RecentOverlap,
    StateMtime,
    EditorEnv,
    FolderHandler,
    InstallRecency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ranked {
    pub index: usize,
    pub step: RankStep,
}

/// How many of an editor's recent projects are repositories we have actually indexed.
///
/// R2: the platform is an argument. A recents list is written by an editor running on this
/// host, so the host's own comparison rules are the right ones. An in-distro path recorded in
/// its UNC display form therefore will not match — that under-counts the overlap, which is the
/// safe direction for a heuristic: it never attributes one editor's evidence to another.
#[must_use]
pub fn overlap(recent: &[PathBuf], known: &BTreeSet<Vec<u8>>) -> usize {
    recent
        .iter()
        .filter(|p| known.contains(&crate::paths::path_key(p, native_platform())))
        .count()
}

/// Returns the single best candidate, or `None`. A tie is not a hit.
fn strict_max<F: Fn(&EditorEvidence) -> Option<i64>>(
    editors: &[EditorEvidence],
    score: F,
) -> Option<usize> {
    let mut best: Option<(usize, i64)> = None;
    let mut tied = false;
    for editor in editors {
        let Some(value) = score(editor) else { continue };
        match best {
            Some((_, current)) if value < current => {}
            Some((_, current)) if value == current => tied = true,
            _ => {
                best = Some((editor.index, value));
                tied = false;
            }
        }
    }
    if tied {
        None
    } else {
        best.map(|(index, _)| index)
    }
}

/// One step's score for one candidate. `None` means this step has nothing to say about it.
type Score<'a> = &'a dyn Fn(&EditorEvidence) -> Option<i64>;

#[must_use]
pub fn rank_editors(inputs: &RankInputs<'_>) -> Option<Ranked> {
    if let [only] = inputs.editors {
        return Some(Ranked {
            index: only.index,
            step: RankStep::RecentOverlap,
        });
    }
    let steps: [(RankStep, Score<'_>); 5] = [
        (RankStep::RecentOverlap, &|e: &EditorEvidence| {
            let n = overlap(&e.recent_paths, inputs.known_locations);
            if n == 0 {
                None
            } else {
                i64::try_from(n).ok()
            }
        }),
        (RankStep::StateMtime, &|e: &EditorEvidence| e.state_mtime),
        (RankStep::EditorEnv, &|e: &EditorEvidence| {
            inputs
                .editor_env_stem
                .filter(|s| s.eq_ignore_ascii_case(&e.stem))
                .map(|_| 1)
        }),
        (RankStep::FolderHandler, &|e: &EditorEvidence| {
            inputs
                .folder_handler_stem
                .filter(|s| s.eq_ignore_ascii_case(&e.stem))
                .map(|_| 1)
        }),
        (RankStep::InstallRecency, &|e: &EditorEvidence| {
            e.installed_at
        }),
    ];
    for (step, score) in steps {
        if let Some(index) = strict_max(inputs.editors, score) {
            return Some(Ranked { index, step });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )]

    use super::*;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn known(paths: &[&str]) -> BTreeSet<Vec<u8>> {
        paths
            .iter()
            .map(|p| crate::paths::path_key(std::path::Path::new(p), native_platform()))
            .collect()
    }

    fn ed(index: usize, stem: &str, recent: &[&str]) -> EditorEvidence {
        EditorEvidence {
            index,
            stem: stem.to_owned(),
            recent_paths: recent.iter().map(PathBuf::from).collect(),
            state_mtime: None,
            installed_at: None,
        }
    }

    #[test]
    fn the_editor_that_has_actually_opened_your_repositories_wins() {
        let editors = [
            ed(0, "alpha", &["/code/one"]),
            ed(1, "beta", &["/code/one", "/code/two", "/code/three"]),
        ];
        let known = known(&["/code/one", "/code/two", "/code/three"]);
        let r = rank_editors(&RankInputs {
            editors: &editors,
            known_locations: &known,
            editor_env_stem: Some("alpha"),
            folder_handler_stem: None,
        })
        .unwrap();
        assert_eq!(r.index, 1);
        assert_eq!(r.step, RankStep::RecentOverlap, "step 1 beats step 3");
    }

    #[test]
    fn a_recent_entry_that_is_not_a_repository_we_found_counts_for_nothing() {
        let editors = [
            ed(0, "alpha", &["/elsewhere/x", "/elsewhere/y"]),
            ed(1, "beta", &["/code/one"]),
        ];
        let known = known(&["/code/one"]);
        let r = rank_editors(&RankInputs {
            editors: &editors,
            known_locations: &known,
            editor_env_stem: None,
            folder_handler_stem: None,
        })
        .unwrap();
        assert_eq!(r.index, 1);
    }

    #[test]
    fn a_tie_on_overlap_falls_through_to_the_next_step() {
        let mut editors = [
            ed(0, "alpha", &["/code/one"]),
            ed(1, "beta", &["/code/one"]),
        ];
        editors[1].state_mtime = Some(200);
        editors[0].state_mtime = Some(100);
        let known = known(&["/code/one"]);
        let r = rank_editors(&RankInputs {
            editors: &editors,
            known_locations: &known,
            editor_env_stem: None,
            folder_handler_stem: None,
        })
        .unwrap();
        assert_eq!((r.index, r.step), (1, RankStep::StateMtime));
    }

    #[test]
    fn the_environment_then_the_folder_handler_then_install_recency() {
        let mut editors = [ed(0, "alpha", &[]), ed(1, "beta", &[])];
        let empty = BTreeSet::new();
        let r = rank_editors(&RankInputs {
            editors: &editors,
            known_locations: &empty,
            editor_env_stem: Some("beta"),
            folder_handler_stem: Some("alpha"),
        })
        .unwrap();
        assert_eq!((r.index, r.step), (1, RankStep::EditorEnv));

        let r = rank_editors(&RankInputs {
            editors: &editors,
            known_locations: &empty,
            editor_env_stem: None,
            folder_handler_stem: Some("alpha"),
        })
        .unwrap();
        assert_eq!((r.index, r.step), (0, RankStep::FolderHandler));

        editors[1].installed_at = Some(500);
        editors[0].installed_at = Some(400);
        let r = rank_editors(&RankInputs {
            editors: &editors,
            known_locations: &empty,
            editor_env_stem: None,
            folder_handler_stem: None,
        })
        .unwrap();
        assert_eq!((r.index, r.step), (1, RankStep::InstallRecency));
    }

    #[test]
    fn no_evidence_at_all_ranks_nothing_rather_than_picking_the_first() {
        let editors = [ed(0, "alpha", &[]), ed(1, "beta", &[])];
        assert_eq!(
            rank_editors(&RankInputs {
                editors: &editors,
                known_locations: &BTreeSet::new(),
                editor_env_stem: None,
                folder_handler_stem: None,
            }),
            None
        );
    }

    #[test]
    fn a_single_candidate_is_the_answer_without_needing_evidence() {
        let editors = [ed(7, "only", &[])];
        let r = rank_editors(&RankInputs {
            editors: &editors,
            known_locations: &BTreeSet::new(),
            editor_env_stem: None,
            folder_handler_stem: None,
        })
        .unwrap();
        assert_eq!(r.index, 7);
    }
}

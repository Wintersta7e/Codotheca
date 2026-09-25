//! Turning what the three files said into the rows the roots screen draws.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::firstrun::classify::{self, RootTrait, DISTRO_HOME_DISPLAY};
use crate::firstrun::sources::{self, SourceHit, SourceKind};
use crate::index::path::PathPlatform;
use crate::protocol::{LocationKind, RootProvenance, RootSuggestion};
use crate::wsl::distros::DistroState; // R9: plan 18 owns the distro facts.

/// A suggested row, with the path kept core-side. §2.5: the renderer receives `path_display`
/// and never a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggested {
    pub path: PathBuf,
    pub row: RootSuggestion,
}

/// §10.1a's conventional locations, used **only to fill gaps**. Order is the order they are
/// offered in.
pub const CONVENTION_DIRS: [&str; 6] = ["src", "dev", "code", "git", "repos", "projects"];

/// What a hit contributes as a root, or nothing when it would contribute a refusable one.
///
/// `home` is an argument because both absolute refusals of §10.1a that a *suggestion* can walk
/// into are shape refusals on the container — a filesystem root, and the home directory with no
/// narrowing subdirectory — and a function that can only see one of them splits one rule across
/// two owners.
#[must_use]
pub fn container_for(hit: &SourceHit, home: &Path) -> Option<PathBuf> {
    let candidate = if hit.is_container {
        hit.path.clone()
    } else {
        hit.path.parent()?.to_path_buf()
    };
    // A filesystem root has no parent. §10.1a refuses it, so it is never offered.
    candidate.parent()?;
    if candidate == home {
        // A home directory with no narrowing subdirectory is refused (§10.1a), so it is never
        // offered either.
        return None;
    }
    Some(candidate)
}

/// Everything `assemble` needs that is not a hit.
#[derive(Debug)]
pub struct SuggestInputs<'a> {
    pub env: &'a sources::SourceEnv,
    pub classifier: &'a dyn classify::RootClassifier,
    pub distros: &'a dyn classify::DistroProbe,
    pub platform: PathPlatform,
}

/// The convention directories that exist under this home and are worth offering.
#[must_use]
pub fn existing_conventions(home: &Path, exists: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    CONVENTION_DIRS
        .iter()
        .map(|name| home.join(name))
        .filter(|p| exists(p))
        .collect()
}

/// The rows, ordered: sourced rows by hit count descending then path, then convention rows,
/// then distro rows.
#[must_use]
pub fn assemble(
    hits: &[SourceHit],
    conventions: &[PathBuf],
    inputs: &SuggestInputs<'_>,
) -> Vec<Suggested> {
    let mut counts: BTreeMap<PathBuf, (u32, SourceKind)> = BTreeMap::new();
    for hit in hits {
        let Some(container) = container_for(hit, &inputs.env.home) else {
            continue;
        };
        let entry = counts.entry(container).or_insert((0, hit.kind));
        entry.0 = entry.0.saturating_add(1);
        // The strongest provenance wins the slot: a declaration in a git config outranks a
        // recent-project list, because it is the user saying where repositories live.
        if hit.kind < entry.1 {
            entry.1 = hit.kind;
        }
    }

    let mut sourced: Vec<Suggested> = counts
        .iter()
        .map(|(path, (hits, kind))| {
            let base = match kind {
                SourceKind::GitConfig => RootProvenance::Gitconfig,
                SourceKind::VsCode | SourceKind::JetBrains => RootProvenance::EditorRecent,
            };
            row_for(
                path,
                base,
                Some(kind.detail().to_owned()),
                Some(*hits),
                inputs,
            )
        })
        .collect();
    sourced.sort_by(|a, b| {
        b.row
            .hits
            .cmp(&a.row.hits)
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut out = sourced;
    for path in conventions {
        if out.iter().any(|s| &s.path == path) {
            continue;
        }
        out.push(row_for(
            path,
            RootProvenance::Convention,
            None,
            None,
            inputs,
        ));
    }

    for distro in inputs.distros.distros() {
        out.push(Suggested {
            // R9: derived here, never carried in plan 18's facts struct.
            path: PathBuf::from(DISTRO_HOME_DISPLAY),
            row: RootSuggestion {
                path_display: DISTRO_HOME_DISPLAY.to_owned(),
                kind: LocationKind::Wsl,
                distro: distro.name.clone(),
                provenance: RootProvenance::Distro,
                provenance_detail: Some(
                    match distro.state {
                        DistroState::Running => "running",
                        DistroState::Stopped => "stopped",
                    }
                    .to_owned(),
                ),
                hits: None,
                // §13: the tick is the consent to start a stopped distro, so it is never given
                // in advance — and a running distro is not pre-ticked either, because its
                // repositories were never walked and nothing has claimed they exist.
                pre_ticked: false,
            },
        });
    }

    out
}

fn row_for(
    path: &Path,
    base: RootProvenance,
    detail: Option<String>,
    hits: Option<u32>,
    inputs: &SuggestInputs<'_>,
) -> Suggested {
    let (provenance, pre_ticked, detail) = match inputs.classifier.classify(path) {
        RootTrait::CloudSynced => (RootProvenance::CloudSynced, false, detail),
        RootTrait::SystemRoot => (RootProvenance::SystemRoot, false, detail),
        RootTrait::Ordinary => (base, true, detail),
    };
    Suggested {
        path: path.to_path_buf(),
        row: RootSuggestion {
            path_display: crate::paths::path_display(path),
            kind: match inputs.platform {
                PathPlatform::Windows => LocationKind::Win,
                PathPlatform::Unix => LocationKind::Linux,
            },
            distro: String::new(),
            provenance,
            provenance_detail: detail,
            hits,
            pre_ticked,
        },
    }
}

/// The whole of `roots.suggest`: read the three files, fill gaps by convention, classify.
#[must_use]
pub fn suggest(inputs: &SuggestInputs<'_>) -> Vec<Suggested> {
    let hits = sources::collect_hits(inputs.env);
    let conventions = existing_conventions(&inputs.env.home, &|p| p.is_dir());
    assemble(&hits, &conventions, inputs)
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
    use crate::firstrun::classify::{
        DistroProbe, FixedClassifier, NoDistros, RootTrait, DISTRO_HOME_DISPLAY,
    };
    use crate::firstrun::sources::{SourceEnv, SourceHit, SourceKind};
    use crate::index::path::PathPlatform;
    use crate::protocol::RootProvenance;
    use crate::wsl::distros::{DistroInfo, DistroState}; // R9
    use std::path::{Path, PathBuf};

    fn env() -> SourceEnv {
        SourceEnv {
            home: PathBuf::from("/home/u"),
            app_data: None,
            xdg_config: None,
        }
    }

    fn hit(kind: SourceKind, path: &str, is_container: bool) -> SourceHit {
        SourceHit {
            kind,
            path: PathBuf::from(path),
            is_container,
        }
    }

    fn inputs<'a>(
        e: &'a SourceEnv,
        c: &'a FixedClassifier,
        d: &'a dyn DistroProbe,
    ) -> SuggestInputs<'a> {
        SuggestInputs {
            env: e,
            classifier: c,
            distros: d,
            platform: PathPlatform::Unix,
        }
    }

    #[test]
    fn an_editor_entry_contributes_its_parent_and_a_gitdir_contributes_itself() {
        let home = Path::new("/home/u");
        assert_eq!(
            container_for(&hit(SourceKind::VsCode, "/home/u/dev/one", false), home),
            Some(PathBuf::from("/home/u/dev"))
        );
        assert_eq!(
            container_for(&hit(SourceKind::GitConfig, "/home/u/work", true), home),
            Some(PathBuf::from("/home/u/work"))
        );
        // A repository directly in the home directory would make the home the root, which is
        // the refusal §10.1a names. It contributes nothing rather than a refusable row.
        assert_eq!(
            container_for(&hit(SourceKind::VsCode, "/home/u/one", false), home),
            None
        );
    }

    #[test]
    fn hits_count_provenance_and_rows_sort_by_it() {
        let e = env();
        let c = FixedClassifier::new(vec![]);
        let rows = assemble(
            &[
                hit(SourceKind::VsCode, "/home/u/dev/a", false),
                hit(SourceKind::VsCode, "/home/u/dev/b", false),
                hit(SourceKind::JetBrains, "/home/u/src/c", false),
            ],
            &[],
            &inputs(&e, &c, &NoDistros),
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].path, PathBuf::from("/home/u/dev"));
        assert_eq!(rows[0].row.hits, Some(2));
        assert_eq!(rows[1].row.hits, Some(1));
        assert!(rows.iter().all(|r| r.row.pre_ticked));
    }

    // §10.1b: `—` where no source named the row. Never `0`.
    #[test]
    fn a_convention_row_has_no_hit_count_at_all() {
        let e = env();
        let c = FixedClassifier::new(vec![]);
        let rows = assemble(
            &[],
            &[PathBuf::from("/home/u/src")],
            &inputs(&e, &c, &NoDistros),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row.provenance, RootProvenance::Convention);
        assert_eq!(
            rows[0].row.hits, None,
            "a convention row must not report a fabricated count"
        );
        assert!(
            rows[0].row.pre_ticked,
            "criterion 12: first run asks zero configuration questions"
        );
    }

    #[test]
    fn a_convention_row_is_dropped_when_a_source_already_named_it() {
        let e = env();
        let c = FixedClassifier::new(vec![]);
        let rows = assemble(
            &[hit(SourceKind::VsCode, "/home/u/src/a", false)],
            &[PathBuf::from("/home/u/src")],
            &inputs(&e, &c, &NoDistros),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row.provenance, RootProvenance::EditorRecent);
    }

    // §10.1a: cloud-sync roots are detected, shown unchecked, and labelled.
    #[test]
    fn a_cloud_synced_row_arrives_unticked_and_wears_the_reason() {
        let e = env();
        let c = FixedClassifier::new(vec![(
            PathBuf::from("/home/u/synced"),
            RootTrait::CloudSynced,
        )]);
        let rows = assemble(
            &[hit(SourceKind::VsCode, "/home/u/synced/a", false)],
            &[],
            &inputs(&e, &c, &NoDistros),
        );
        assert_eq!(rows[0].row.provenance, RootProvenance::CloudSynced);
        assert!(!rows[0].row.pre_ticked);
    }

    #[test]
    fn a_system_row_arrives_unticked() {
        let e = env();
        let c = FixedClassifier::new(vec![(PathBuf::from("/opt/work"), RootTrait::SystemRoot)]);
        let rows = assemble(
            &[hit(SourceKind::VsCode, "/opt/work/a", false)],
            &[],
            &inputs(&e, &c, &NoDistros),
        );
        assert_eq!(rows[0].row.provenance, RootProvenance::SystemRoot);
        assert!(!rows[0].row.pre_ticked);
    }

    // §13: starting a stopped distro is an explicit, consented action. The tick *is* that
    // consent, so the row must never arrive pre-ticked.
    #[derive(Debug)]
    struct OneStoppedDistro;
    impl DistroProbe for OneStoppedDistro {
        fn distros(&self) -> Vec<DistroInfo> {
            vec![DistroInfo {
                name: "alpha".to_owned(),
                state: DistroState::Stopped,
            }]
        }
    }

    #[test]
    fn a_distro_row_is_never_pre_ticked() {
        let e = env();
        let c = FixedClassifier::new(vec![]);
        let rows = assemble(&[], &[], &inputs(&e, &c, &OneStoppedDistro));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row.provenance, RootProvenance::Distro);
        assert_eq!(rows[0].row.distro, "alpha");
        assert_eq!(rows[0].row.kind, LocationKind::Wsl);
        // R9: the display string is derived, and §8.5's Linux form carries no distro name.
        assert_eq!(rows[0].row.path_display, DISTRO_HOME_DISPLAY);
        assert_eq!(rows[0].row.provenance_detail.as_deref(), Some("stopped"));
        assert!(!rows[0].row.pre_ticked);
    }

    // §2.5 forbids the renderer comparing paths, so a refusable row must never be suggested.
    #[test]
    fn a_refusable_path_is_never_suggested() {
        let e = env();
        let c = FixedClassifier::new(vec![]);
        let rows = assemble(
            &[
                hit(SourceKind::GitConfig, "/", true),
                hit(SourceKind::GitConfig, "/home/u", true),
                hit(SourceKind::GitConfig, "/home/u/work", true),
            ],
            &[],
            &inputs(&e, &c, &NoDistros),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, PathBuf::from("/home/u/work"));
    }

    #[test]
    fn conventions_are_offered_only_where_they_exist() {
        let present = |p: &Path| p.ends_with("src") || p.ends_with("code");
        let found = existing_conventions(Path::new("/home/u"), &present);
        assert_eq!(
            found,
            vec![PathBuf::from("/home/u/src"), PathBuf::from("/home/u/code")]
        );
    }
}

//! §31.4's archetype proposal, **in both vocabularies** (R130/F10).
//!
//! §30.3's `notApplicable` outcome is over [`DebtSource`] variants and cannot read a table keyed
//! on [`CompletionCheck`]. Stating the proposal in one vocabulary alone is how the health plan
//! ends up writing an unspecified translation of it — or shipping `missing_tests` debt on a
//! documentation project.

use crate::protocol::{CompletionCheck, DebtSource};

/// Does this project's archetype propose that this check does not apply?
///
/// | Archetype | Proposes `na` for |
/// |---|---|
/// | `docs`, `config` | `tests`, `ci`, `deps`, `release` |
/// | every other archetype, **including `unclassified`** | nothing |
///
/// Two rules produce that table, and a later editor must argue against them rather than around
/// them:
///
/// - **A proposal is made only where the check is structurally meaningless**, never where it is
///   merely uncommon. A proposal shrinks the denominator and a shrunk denominator is easier to
///   fill, so the safe default is to propose rarely; the user can always add one, and
///   `projects.setCheckNa` is how.
/// - **`unclassified` proposes nothing.** An unclassified archetype is an *absence of evidence*,
///   and proposing N/A from absence of evidence is rendering unknown as zero **on the
///   denominator itself**.
///
/// `None` is a NULL `archetype` column — **J3 has not run** — and proposes nothing, for the same
/// reason and one step earlier.
///
/// **The farm is left open**, and no cap, floor or minimum is introduced: marking nine checks
/// N/A and passing the tenth reaches notched gold. `concept.md:219-221` settles it — gaming is
/// accepted, not engineered against. Recorded so a later reviewer does not "fix" it.
#[must_use]
pub fn proposes_na(archetype: Option<&str>, key: CompletionCheck) -> bool {
    let Some(archetype) = archetype else {
        return false;
    };
    if !matches!(archetype, "docs" | "config") {
        return false;
    }
    matches!(
        key,
        CompletionCheck::Tests
            | CompletionCheck::Ci
            | CompletionCheck::Deps
            | CompletionCheck::Release
    )
}

/// §31.9's bijection from a check key to the debt source that stands behind it.
///
/// **Bijection, not identity, and the two are never conflated.** A4's registry names
/// `missing_readme · missing_license · missing_tests · unpushed_commits · ci_red · no_release`;
/// §31's keys are `readme · license · tests · pushed · ciGreen · release`. §28 owns which
/// spelling the column holds. The same mismatch is correct one level up: the dependency
/// *switch* is named for the source, `dependency_advisory`, while the dependency *check* is
/// `deps`.
///
/// `None` for `remote`, `description` and `ci` — those three own **no item at all** under A4's
/// registry, which is why §30's second column carries a dash where `ci` would be. `ci_red` is
/// proposed `na` only derivatively, through §31.1a's *`ciGreen` is `na` when `ci` is `na` or
/// `fail`*, which is a `CompletionCheck` rule §30 never reads.
///
/// `deps` maps to `dependency_advisory` although it **owns** no item: it reads §32's set, and the
/// source is still what §30 must suppress when the check is not applicable.
#[must_use]
pub const fn suppressed_source(key: CompletionCheck) -> Option<DebtSource> {
    match key {
        CompletionCheck::Readme => Some(DebtSource::MissingReadme),
        CompletionCheck::License => Some(DebtSource::MissingLicense),
        CompletionCheck::Tests => Some(DebtSource::MissingTests),
        CompletionCheck::Pushed => Some(DebtSource::UnpushedCommits),
        CompletionCheck::CiGreen => Some(DebtSource::CiRed),
        CompletionCheck::Release => Some(DebtSource::NoRelease),
        CompletionCheck::Deps => Some(DebtSource::DependencyAdvisory),
        CompletionCheck::Remote | CompletionCheck::Description | CompletionCheck::Ci => None,
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
    use crate::jobs::classify::ARCHETYPES;
    use std::collections::BTreeSet;

    #[test]
    fn only_docs_and_config_propose_anything_and_unclassified_proposes_nothing() {
        let mut proposing = BTreeSet::new();
        for archetype in ARCHETYPES {
            for key in CompletionCheck::ALL {
                if proposes_na(Some(archetype), key) {
                    proposing.insert(archetype);
                }
            }
        }
        assert_eq!(
            proposing,
            BTreeSet::from(["config", "docs"]),
            "a proposal is made only where the check is structurally meaningless"
        );
        for key in CompletionCheck::ALL {
            assert!(!proposes_na(Some("unclassified"), key));
            // A NULL column is J3 not having run, which is one step earlier and the same rule.
            assert!(!proposes_na(None, key));
        }
    }

    #[test]
    fn docs_and_config_propose_exactly_the_four_structurally_meaningless_checks() {
        for archetype in ["docs", "config"] {
            let proposed: Vec<CompletionCheck> = CompletionCheck::ALL
                .into_iter()
                .filter(|k| proposes_na(Some(archetype), *k))
                .collect();
            assert_eq!(
                proposed,
                vec![
                    CompletionCheck::Tests,
                    CompletionCheck::Ci,
                    CompletionCheck::Deps,
                    CompletionCheck::Release,
                ],
                "{archetype}"
            );
        }
    }

    /// The map is total over the ten keys and injective over the seven that have a source, so a
    /// key cannot silently share another's item.
    #[test]
    fn the_bijection_is_total_and_injective() {
        let mut sources = BTreeSet::new();
        let mut without = Vec::new();
        for key in CompletionCheck::ALL {
            match suppressed_source(key) {
                Some(source) => assert!(
                    sources.insert(format!("{source:?}")),
                    "{key:?} shares a source with another check"
                ),
                None => without.push(key),
            }
        }
        assert_eq!(
            without,
            vec![
                CompletionCheck::Remote,
                CompletionCheck::Description,
                CompletionCheck::Ci
            ],
            "§31.9: exactly three checks own no item at all"
        );
        assert_eq!(sources.len(), 7);
    }
}

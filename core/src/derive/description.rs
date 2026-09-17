//! §5.2 — the description chain.

/// Where a project's description came from.
///
/// **R31: declared in `protocol/schema/protocol.json` and generated into `crate::protocol`.**
/// Re-exported so this module's path names it, and hand-written nowhere: the value is stored in
/// `project.description_source`, whose CHECK lists the same four words, and a second copy would
/// compile and then drift from the one the wire and the column agree on.
pub use crate::protocol::DescriptionSource;

impl DescriptionSource {
    /// The stored form, matching `description_source`'s CHECK character for character.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            DescriptionSource::Manifest => "manifest",
            DescriptionSource::Readme => "readme",
            DescriptionSource::Note => "note",
            DescriptionSource::Detected => "detected",
            DescriptionSource::Remote => "remote",
        }
    }
}

/// A description and the evidence for it. Both `None` means nothing is known — which the
/// surface renders by saying nothing, never by inventing a line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Described {
    /// The text, `None` when nothing was found.
    pub text: Option<String>,
    /// Which rung of the chain produced it.
    pub source: Option<DescriptionSource>,
}

fn is_badge(line: &str) -> bool {
    let t = line.trim();
    t.starts_with("[![") || t.starts_with("![") || (t.starts_with('[') && t.contains("](http"))
}

/// A raw HTML line, which is never a description.
///
/// `is_badge` knows only markdown forms, so a README that centres its header with
/// `<div align="center">` — the common shape, not an exotic one — put that tag verbatim into the
/// project's summary line under the title. §25.5 renders raw HTML as visible text by design
/// (`markdown-it` with `html: false`), so nothing downstream strips it either.
fn is_markup(line: &str) -> bool {
    line.trim_start().starts_with('<')
}

/// The H1 subtitle — the line directly under a `# ` heading — or the first sentence that is
/// not a badge row.
#[must_use]
pub fn readme_description(readme: &str) -> Option<String> {
    let mut lines = readme.lines().peekable();
    if lines
        .peek()
        .is_some_and(|l| l.trim_start().starts_with("# "))
    {
        lines.next();
        if let Some(next) = lines.peek() {
            let t = next.trim();
            if !t.is_empty() && !is_badge(t) && !is_markup(t) && !t.starts_with('#') {
                return Some(t.to_owned());
            }
        }
    }
    readme
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !is_badge(l) && !is_markup(l) && !l.starts_with('#'))
        .map(|l| {
            l.split_once(". ")
                .map_or_else(|| l.to_owned(), |(s, _)| format!("{s}."))
        })
}

/// `Rust CLI`, and nothing that was not detected.
///
/// `unclassified` and an unknown language both yield `None`: §5.2's own example carries a
/// dependency name, and phase 1 has no framework-recognition table — printing whichever
/// manifest entry happened to be first would characterise a project by an accident. A
/// synthesised description that names neither the language nor the shape is furniture.
#[must_use]
pub fn synthesise(primary_language: Option<&str>, archetype: Option<&str>) -> Option<String> {
    let lang = primary_language?;
    let arch = archetype?;
    let word = match arch {
        "cli" => "CLI",
        "library" => "library",
        "service" => "service",
        "site" => "site",
        "notebook" => "notebook",
        "docs" => "documentation",
        "config" => "configuration",
        _ => return None,
    };
    Some(format!("{lang} {word}"))
}

/// §5.2's chain: manifest, then the forge, then README, then the user's note, then synthesis.
///
/// [p2] §25.3 inserts `remote` at **rank 2**, and the two boundaries are the whole of the
/// argument. Below `manifest`, because an in-repo, versioned, offline-available string must not
/// be displaced by a network fact — *GitHub is additive, never a gate*. Above `README`, because
/// the forge's About line is an **authored** one-line summary of exactly this project rather
/// than a sentence extracted heuristically, and for a project that was never cloned it is the
/// only description that exists.
///
/// **The note's rank moves from third to fourth**, which supersedes §8.5.4's sentence.
#[must_use]
pub fn describe(
    manifest: Option<&str>,
    remote: Option<&str>,
    readme: Option<&str>,
    note: Option<&str>,
    primary_language: Option<&str>,
    archetype: Option<&str>,
) -> Described {
    if let Some(text) = manifest.map(str::trim).filter(|t| !t.is_empty()) {
        return Described {
            text: Some(text.to_owned()),
            source: Some(DescriptionSource::Manifest),
        };
    }
    if let Some(text) = remote.map(str::trim).filter(|t| !t.is_empty()) {
        return Described {
            text: Some(text.to_owned()),
            source: Some(DescriptionSource::Remote),
        };
    }
    if let Some(text) = readme.and_then(readme_description) {
        return Described {
            text: Some(text),
            source: Some(DescriptionSource::Readme),
        };
    }
    if let Some(text) = note
        .and_then(|n| n.lines().next())
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        return Described {
            text: Some(text.to_owned()),
            source: Some(DescriptionSource::Note),
        };
    }
    match synthesise(primary_language, archetype) {
        Some(text) => Described {
            text: Some(text),
            source: Some(DescriptionSource::Detected),
        },
        None => Described::default(),
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

    #[test]
    fn the_chain_prefers_a_manifest_description() {
        let d = describe(
            Some("A tiny thing"),
            Some("The forge's About line"),
            Some("# Title\n\nProse."),
            Some("note"),
            Some("Rust"),
            Some("cli"),
        );
        assert_eq!(d.text.as_deref(), Some("A tiny thing"));
        assert_eq!(d.source, Some(DescriptionSource::Manifest));
    }

    #[test]
    fn badge_lines_are_not_the_first_sentence() {
        let readme = "# Thing\n\n[![build](https://x/y.svg)](https://x)\n\nIt indexes things.\n";
        assert_eq!(
            readme_description(readme).as_deref(),
            Some("It indexes things.")
        );
    }

    /// Measured in the built app: a project's summary line under its title read
    /// `<div align="center">` verbatim. `is_badge` knows only markdown forms, so the HTML wrapper
    /// a centred README header opens with passed every filter and became the description.
    #[test]
    fn an_html_wrapper_is_not_the_description() {
        let readme = "<div align=\"center\">\n\n# Thing\n\nIt indexes things.\n\n</div>\n";
        assert_eq!(
            readme_description(readme).as_deref(),
            Some("It indexes things.")
        );
    }

    /// The same shape one line further in: an HTML badge row is a badge, and `is_badge` cannot
    /// see it because it carries no `![`.
    #[test]
    fn an_html_badge_row_is_not_the_description() {
        let readme = "# Thing\n<img src=\"https://img.example/badge.svg\" alt=\"build\">\n\nIt indexes things.\n";
        assert_eq!(
            readme_description(readme).as_deref(),
            Some("It indexes things.")
        );
    }

    /// The counter-case, so the filter is a filter and not a blanket: prose that merely mentions
    /// a comparison still describes the project.
    #[test]
    fn prose_that_is_not_a_tag_survives() {
        let readme = "# Thing\nIndexes 10 < 20 repositories at a time.\n";
        assert_eq!(
            readme_description(readme).as_deref(),
            Some("Indexes 10 < 20 repositories at a time.")
        );
    }

    #[test]
    fn an_h1_subtitle_wins_over_a_later_sentence() {
        let readme = "# Thing\nA library for one job.\n\nInstall it like this.\n";
        assert_eq!(
            readme_description(readme).as_deref(),
            Some("A library for one job.")
        );
    }

    #[test]
    fn the_note_first_line_is_used_before_synthesis() {
        let d = describe(
            None,
            None,
            None,
            Some("my scratch pad\nsecond line"),
            Some("Rust"),
            Some("cli"),
        );
        assert_eq!(d.text.as_deref(), Some("my scratch pad"));
        assert_eq!(d.source, Some(DescriptionSource::Note));
    }

    #[test]
    fn synthesis_states_only_what_was_detected() {
        // §5.2's example carries a dependency name. Phase 1 has no framework-recognition
        // table, and printing an arbitrary manifest entry would characterise a project by
        // whichever dependency happened to be first. Language and archetype, or nothing.
        assert_eq!(
            synthesise(Some("Rust"), Some("cli")).as_deref(),
            Some("Rust CLI")
        );
        assert_eq!(
            synthesise(Some("Python"), Some("library")).as_deref(),
            Some("Python library")
        );
        assert_eq!(synthesise(None, Some("cli")), None);
        assert_eq!(synthesise(Some("Rust"), Some("unclassified")), None);
    }

    #[test]
    fn nothing_known_describes_nothing_and_says_so_by_being_absent() {
        let d = describe(None, None, None, None, None, None);
        assert_eq!(d.text, None);
        assert_eq!(d.source, None);
    }

    /// The slug is stored in `project.description_source`, whose CHECK lists the same five
    /// words after `0009` widened it. `core/tests/index_rebuild_0009.rs` compares the CHECK's
    /// literals against the schema's variant set in both directions, against a real column.
    ///
    /// **`remote` has a slug and no rung.** §25.3's chain — manifest, remote, README, note,
    /// detected — is p2-25's half; this plan lands the value the column and the wire accept.
    #[test]
    fn every_source_has_a_slug_and_the_wire_form_agrees() {
        for (source, slug) in [
            (DescriptionSource::Manifest, "manifest"),
            (DescriptionSource::Readme, "readme"),
            (DescriptionSource::Note, "note"),
            (DescriptionSource::Detected, "detected"),
            (DescriptionSource::Remote, "remote"),
        ] {
            assert_eq!(source.slug(), slug);
            assert_eq!(
                serde_json::to_string(&source).unwrap(),
                format!("\"{slug}\""),
                "the stored slug and the wire form are one value"
            );
        }
    }
}

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
            if !t.is_empty() && !is_badge(t) && !t.starts_with('#') {
                return Some(t.to_owned());
            }
        }
    }
    readme
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !is_badge(l) && !l.starts_with('#'))
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

/// §5.2's chain: manifest, then README, then the user's note, then synthesis, then nothing.
#[must_use]
pub fn describe(
    manifest: Option<&str>,
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
        let d = describe(None, None, None, None, None);
        assert_eq!(d.text, None);
        assert_eq!(d.source, None);
    }

    /// The slug is stored in `project.description_source`, whose CHECK lists the same four
    /// words. `core/tests/jobs_content.rs` inserts every one of them against the real column.
    #[test]
    fn every_source_has_a_slug_and_the_wire_form_agrees() {
        for (source, slug) in [
            (DescriptionSource::Manifest, "manifest"),
            (DescriptionSource::Readme, "readme"),
            (DescriptionSource::Note, "note"),
            (DescriptionSource::Detected, "detected"),
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

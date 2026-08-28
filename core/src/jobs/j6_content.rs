//! J6 — content (§4.1). Byte-capped and cancellable; on exceed the field is omitted, never
//! truncated into a claim.

use std::io::Read as _;
use std::path::Path;

/// §4.1's cap on any one file J6 reads.
pub const J6_BYTE_CAP: usize = 256 * 1024;

const README_NAMES: &[&str] = &[
    "README.md",
    "README.rst",
    "README.txt",
    "README",
    "readme.md",
];
const MANIFEST_NAMES: &[&str] = &["Cargo.toml", "package.json", "pyproject.toml"];

/// What J6 found in the working tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentFacts {
    /// A `description` field lifted from a manifest, if there was one.
    pub manifest_description: Option<String>,
    /// Which README was read.
    pub readme_path: Option<String>,
    /// Its contents, capped.
    pub readme_excerpt: Option<String>,
    /// True once J6 has looked.
    ///
    /// §8.4 draws two different sentences: `readme_seen && readme_path.is_none()` is "No README
    /// in this repository", a fact; `!readme_seen` is "No README indexed yet", a promise. One
    /// string for both would render unknown as zero.
    pub readme_seen: bool,
}

fn read_capped(path: &Path, cap: usize) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::new();
    let mut limited = file.take(u64::try_from(cap).unwrap_or(u64::MAX));
    limited.read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Lines joined with a space up to the first blank line.
///
/// Markdown wraps; a description carrying a hard newline renders as two descriptions.
#[must_use]
pub fn first_paragraph(readme: &str) -> String {
    readme
        .lines()
        .take_while(|l| !l.trim().is_empty())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
}

/// A very small manifest reader: it extracts a `description` field and nothing else.
///
/// Deliberately not a parser for any manifest format — the only value phase 1 needs from a
/// manifest is one string, and a full parse buys nothing and can fail on valid input.
fn manifest_description(text: &str, json: bool) -> Option<String> {
    for line in text.lines() {
        let t = line.trim();
        // A line that is not the description simply is not this line. The plan's version used
        // `?` here, which returns from the whole function on the first non-matching line — so it
        // only ever worked when `description` was line one, which it is in no real manifest.
        let Some(rest) = (if json {
            t.strip_prefix("\"description\"")
                .map(str::trim_start)
                .and_then(|r| r.strip_prefix(':'))
        } else {
            t.strip_prefix("description")
                .map(str::trim_start)
                .and_then(|r| r.strip_prefix('='))
        }) else {
            continue;
        };
        let v = rest.trim().trim_end_matches(',').trim();
        let Some(v) = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
            continue;
        };
        if !v.is_empty() {
            return Some(v.to_owned());
        }
    }
    None
}

/// Read the working tree's README and manifest.
#[must_use]
pub fn read_content(work_dir: &Path, cap: usize) -> ContentFacts {
    let mut facts = ContentFacts {
        readme_seen: true,
        ..ContentFacts::default()
    };

    for name in README_NAMES {
        let path = work_dir.join(name);
        if !path.is_file() {
            continue;
        }
        facts.readme_path = Some((*name).to_owned());
        facts.readme_excerpt = read_capped(&path, cap);
        break;
    }

    for name in MANIFEST_NAMES {
        let path = work_dir.join(name);
        if !path.is_file() {
            continue;
        }
        let Some(text) = read_capped(&path, cap) else {
            continue;
        };
        if let Some(d) = manifest_description(&text, *name == "package.json") {
            facts.manifest_description = Some(d);
            break;
        }
    }

    facts
}

/// Write the description chain's answer and the Peek excerpt.
/// `now` is the caller's, matching every other writer in the tree: the transaction and the
/// clock both belong to whoever opened them, which is what keeps the `Clock` seam out of here.
pub fn persist(
    tx: &rusqlite::Transaction<'_>,
    project: crate::protocol::ProjectId,
    facts: &ContentFacts,
    now: i64,
) -> Result<(), crate::index::IndexError> {
    let (lang, arch, note): (Option<String>, Option<String>, Option<String>) = tx.query_row(
        "SELECT primary_language, archetype, notes FROM project WHERE id = ?1",
        [project.0],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let described = crate::derive::description::describe(
        facts.manifest_description.as_deref(),
        facts.readme_excerpt.as_deref(),
        note.as_deref(),
        lang.as_deref(),
        arch.as_deref(),
    );
    tx.execute(
        "UPDATE project SET description = ?2, description_source = ?3 WHERE id = ?1",
        rusqlite::params![
            project.0,
            described.text,
            described
                .source
                .map(crate::derive::description::DescriptionSource::slug),
        ],
    )?;
    // A row present with a NULL excerpt is "no README in this repository"; a row absent is
    // "no README indexed yet" (§8.4). The two are not one string, so J6 writes the row either
    // way and leaves the column NULL when there was nothing to read.
    tx.execute(
        "INSERT INTO peek_cache (project_id, readme_excerpt, computed_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(project_id) DO UPDATE SET
             readme_excerpt = excluded.readme_excerpt, computed_at = excluded.computed_at",
        rusqlite::params![
            project.0,
            facts.readme_excerpt.as_deref().map(first_paragraph),
            now,
        ],
    )?;
    Ok(())
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
    fn the_read_is_byte_capped() {
        // §4.1 J6: byte-capped 256 KB, cancellable; field omitted on exceed.
        assert_eq!(J6_BYTE_CAP, 262_144);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "x".repeat(J6_BYTE_CAP * 2)).unwrap();
        let facts = read_content(dir.path(), J6_BYTE_CAP);
        assert!(facts.readme_seen);
        let excerpt = facts.readme_excerpt.unwrap_or_default();
        assert!(excerpt.len() <= J6_BYTE_CAP);
    }

    #[test]
    fn an_absent_readme_and_an_unread_one_are_different_states() {
        // §8.4: "No README indexed yet." is a promise; "No README in this repository." is a
        // fact. One string for both would render unknown as zero.
        let dir = tempfile::tempdir().unwrap();
        let facts = read_content(dir.path(), J6_BYTE_CAP);
        assert!(facts.readme_seen, "J6 ran");
        assert_eq!(facts.readme_path, None, "and there is no README");

        let never_ran = ContentFacts::default();
        assert!(!never_ran.readme_seen);
        assert_eq!(never_ran.readme_path, None);
    }

    #[test]
    fn the_first_paragraph_stops_at_the_blank_line() {
        assert_eq!(first_paragraph("one\ntwo\n\nthree\n"), "one two");
    }

    /// The plan's reader bailed out of the whole loop on the first line that was not the
    /// description, so it only worked on a manifest whose very first line was one. No real
    /// manifest looks like that.
    #[test]
    fn a_description_below_other_manifest_lines_is_still_found() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\ndescription = \"Indexes things\"\n",
        )
        .unwrap();
        let facts = read_content(dir.path(), J6_BYTE_CAP);
        assert_eq!(
            facts.manifest_description.as_deref(),
            Some("Indexes things")
        );
    }

    #[test]
    fn a_json_manifest_description_is_found_too() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            "{\n  \"name\": \"x\",\n  \"description\": \"Indexes things\",\n  \"version\": \"1\"\n}\n",
        )
        .unwrap();
        let facts = read_content(dir.path(), J6_BYTE_CAP);
        assert_eq!(
            facts.manifest_description.as_deref(),
            Some("Indexes things")
        );
    }

    #[test]
    fn a_manifest_with_no_description_yields_none_rather_than_an_empty_string() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\ndescription = \"\"\n",
        )
        .unwrap();
        assert_eq!(
            read_content(dir.path(), J6_BYTE_CAP).manifest_description,
            None
        );
    }
}

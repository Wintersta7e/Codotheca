//! The user's own addresses, read from the git config files they already have.
//!
//! §1.4's card is a **confirmation**: "every figure you have just seen was computed from these
//! addresses". That presumes the addresses exist, and `identity::people::seed` is what puts them
//! there — from `user.email` first, and from the scan's own committer tally after that.
//!
//! Until this module existed, `seed` had **no production caller and no input**, so the `identity`
//! table was empty on every real machine. J1.5 then folded every repository's committers against
//! an empty set, wrote `authored_by_user = 0` and `is_reference = 1` for all of them, and §8.0b's
//! base predicate — *a bare query returns no `is_reference` rows* — left `projects.list` answering
//! zero rows over a full index. Measured: three projects in `project`, `stats.reveal` reporting
//! three, and the shelf showing `NOTHING INDEXED YET`.
//!
//! **The files, not `git config`.** The addresses live in a text file this process can read, and
//! spawning git would put a process launch in the startup path for two strings. `includeIf` and
//! the system config are not followed — an address they alone carry is reached by the scan's own
//! tally instead, and by §1.4's card, which exists precisely so the seeded set can be corrected.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Every `user.email` in this user's git configuration, in file order, without duplicates.
///
/// Case is preserved as written; the comparison everything downstream makes is case-insensitive,
/// so `A@B.C` and `a@b.c` are one address and the first spelling wins.
#[must_use]
pub fn user_emails(home: &Path, xdg_config: Option<&Path>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for file in config_files(home, xdg_config) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for email in parse_user_emails(&text) {
            if seen.insert(email.to_lowercase()) {
                out.push(email);
            }
        }
    }
    out
}

/// The two files git reads for a user, in git's own precedence order.
fn config_files(home: &Path, xdg_config: Option<&Path>) -> Vec<PathBuf> {
    let xdg = xdg_config.map_or_else(|| home.join(".config"), Path::to_path_buf);
    vec![xdg.join("git").join("config"), home.join(".gitconfig")]
}

/// `[user] email = …`, from git's config syntax.
///
/// A subsection (`[user "work"]`) is **not** `user`, and a value's inline `#` or `;` comment is
/// not part of the address — both are how a parser that splits on `=` alone invents an identity
/// the user does not have.
#[must_use]
pub fn parse_user_emails(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_user = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(header) = section_header(line) {
            in_user = header.eq_ignore_ascii_case("user");
            continue;
        }
        if !in_user {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("email") {
            continue;
        }
        let value = strip_comment(value);
        let value = value.trim().trim_matches('"').trim();
        if !value.is_empty() {
            out.push(value.to_owned());
        }
    }
    out
}

/// The name inside `[…]`, when the line is a section header and names no subsection.
fn section_header(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('[')?;
    let inner = inner.split(']').next()?;
    if inner.contains('"') {
        // `[user "work"]` is a subsection: its `email` is not `user.email`.
        return Some("");
    }
    Some(inner.trim())
}

fn strip_comment(value: &str) -> &str {
    match value.find(['#', ';']) {
        Some(at) => value.get(..at).unwrap_or(""),
        None => value,
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
    fn the_user_section_is_the_only_one_that_carries_an_address() {
        let text = "[core]\n\temail = not-a-person@example.invalid\n\
                    [user]\n\tname = A Person\n\temail = a@example.invalid\n";
        assert_eq!(
            parse_user_emails(text),
            vec!["a@example.invalid".to_owned()]
        );
    }

    #[test]
    fn a_subsection_is_not_the_user_section() {
        let text = "[user \"work\"]\n\temail = work@example.invalid\n";
        assert!(parse_user_emails(text).is_empty());
    }

    #[test]
    fn a_trailing_comment_is_not_part_of_the_address() {
        let text = "[user]\n\temail = a@example.invalid # the main one\n";
        assert_eq!(
            parse_user_emails(text),
            vec!["a@example.invalid".to_owned()]
        );
    }

    #[test]
    fn a_quoted_value_loses_its_quotes_and_nothing_else() {
        let text = "[user]\n\temail = \"a+tag@example.invalid\"\n";
        assert_eq!(
            parse_user_emails(text),
            vec!["a+tag@example.invalid".to_owned()]
        );
    }

    #[test]
    fn two_files_answer_once_per_address_and_keep_the_first_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        std::fs::create_dir_all(home.join(".config").join("git")).unwrap();
        std::fs::write(
            home.join(".config").join("git").join("config"),
            "[user]\n\temail = A@Example.Invalid\n",
        )
        .unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            "[user]\n\temail = a@example.invalid\n\temail = second@example.invalid\n",
        )
        .unwrap();
        assert_eq!(
            user_emails(home, None),
            vec![
                "A@Example.Invalid".to_owned(),
                "second@example.invalid".to_owned()
            ]
        );
    }

    #[test]
    fn a_home_with_no_git_configuration_answers_nothing_rather_than_failing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(user_emails(dir.path(), None).is_empty());
    }
}

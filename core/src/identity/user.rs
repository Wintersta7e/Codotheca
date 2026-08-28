//! §1.4's identity set — the addresses that count as "the user".
//!
//! **Plan 08 never shipped this**, though plan 09's interface block consumes it from
//! `core::identity` and §4.1a's authorship gate cannot answer "did the user write this?" without
//! it. Declared here rather than inside `core::jobs` because the two tables it reads are this
//! module's, and a set of addresses computed beside its tables is one implementation instead of
//! the two that R12 exists to prevent.

use std::collections::BTreeSet;

use rusqlite::Connection;

use super::IdentityError;

/// Every address that means the user, confirmed identities and their aliases together.
///
/// §1.4: an alias is an address sharing a local part, a co-author trailer, or a manual addition.
/// All of them count as the user for §5.5's authorship question — an address the user commits
/// under is the user whether or not it is the one in `git config`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentitySet {
    /// Lowercased addresses. Comparison is case-insensitive, matching the column's
    /// `UNIQUE (email COLLATE NOCASE)`.
    pub emails: BTreeSet<String>,
}

impl IdentitySet {
    /// Build one from addresses, lowercasing and discarding blanks.
    pub fn from_emails<I: IntoIterator<Item = String>>(emails: I) -> Self {
        Self {
            emails: emails
                .into_iter()
                .map(|e| e.trim().to_lowercase())
                .filter(|e| !e.is_empty())
                .collect(),
        }
    }

    /// Whether an address is the user's. Case-insensitive, and blank is never a match — an
    /// empty committer address is missing data, not the user.
    #[must_use]
    pub fn contains(&self, email: &str) -> bool {
        let e = email.trim().to_lowercase();
        !e.is_empty() && self.emails.contains(&e)
    }

    /// True when the set is empty, which is *not computed* rather than "nobody".
    ///
    /// §5.5 must not answer "somebody else wrote this" from an unseeded set: with no identity
    /// the honest answer is that authorship is unknown, and every repository would otherwise be
    /// classified Reference on first run.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.emails.is_empty()
    }
}

/// Read the identity set out of the index.
///
/// `identity.is_user` excludes an address recorded for someone else; `identity_alias` carries
/// §1.4's three alias reasons and every one of them counts.
pub fn load_identity_set(conn: &Connection) -> Result<IdentitySet, IdentityError> {
    let mut stmt = conn
        .prepare(
            "SELECT email FROM identity WHERE is_user = 1
             UNION
             SELECT a.email FROM identity_alias a
               JOIN identity i ON i.id = a.identity_id
              WHERE i.is_user = 1",
        )
        .map_err(IdentityError::Sqlite)?;
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(IdentityError::Sqlite)?;
    let mut emails = Vec::new();
    for row in rows {
        emails.push(row.map_err(IdentityError::Sqlite)?);
    }
    Ok(IdentitySet::from_emails(emails))
}

//! §1.4's identity set: who the addresses in this library belong to.
//!
//! Seeding is deliberately narrow. The card that renders this set must be answerable at a
//! glance with no keyboard, and an address the seeding missed is not discoverable from a list
//! that does not contain it — so widening belongs to settings, where it is also the safe
//! direction: it can only move projects *out* of Reference.

use std::collections::{BTreeMap, BTreeSet};

use crate::index::IndexError;
use crate::protocol::{AliasReason, IdentityId, IdentityRow, IdentitySource};

/// Hosts whose addresses are a person's own by construction.
const NOREPLY_HOSTS: [&str; 1] = ["users.noreply.github.com"];

/// Every `user.email` in a git config, in file order.
#[must_use]
pub fn parse_config_emails(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut in_user = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_user = line.starts_with("[user]") || line.starts_with("[user ");
            continue;
        }
        if !in_user {
            continue;
        }
        if let Some(rest) = line.strip_prefix("email") {
            let value = rest.trim_start().trim_start_matches('=').trim();
            if !value.is_empty() && !out.iter().any(|e| e.as_str() == value) {
                out.push(value.to_owned());
            }
        }
    }
    out
}

/// The part before the `@`, with any `+tag` removed.
#[must_use]
pub fn local_part(email: &str) -> Option<&str> {
    let at = email.find('@')?;
    let local = email.get(..at)?;
    if local.is_empty() {
        return None;
    }
    Some(local.split('+').next().unwrap_or(local))
}

/// True for an address on a host that issues per-person addresses.
#[must_use]
pub fn is_noreply(email: &str) -> bool {
    email
        .rsplit('@')
        .next()
        .is_some_and(|host| NOREPLY_HOSTS.iter().any(|h| host.eq_ignore_ascii_case(h)))
}

/// How many rows each seeding rule produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SeedReport {
    pub from_config: u32,
    pub noreply: u32,
    pub local_part: u32,
    pub coauthor: u32,
}

/// Seed the identity set. Idempotent: running it again writes nothing new.
///
/// # Errors
/// Returns [`IndexError`] when a read or write fails.
pub fn seed(
    conn: &rusqlite::Connection,
    config_emails: &[String],
    now: i64,
) -> Result<SeedReport, IndexError> {
    let mut report = SeedReport::default();

    for email in config_emails {
        if insert_identity(conn, email, "gitconfig", now)? {
            report.from_config += 1;
        }
    }

    // Everything below is derived from what the scan already tallied, so it costs a join and
    // not a history walk.
    let pairs: Vec<(i64, String)> = {
        let mut stmt = conn.prepare("SELECT project_id, email FROM project_committer")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    let known: BTreeSet<String> = config_emails.iter().map(|e| e.to_lowercase()).collect();
    let known_locals: BTreeSet<String> = config_emails
        .iter()
        .filter_map(|e| local_part(e).map(str::to_lowercase))
        .collect();
    let known_projects: BTreeSet<i64> = pairs
        .iter()
        .filter(|(_, email)| known.contains(&email.to_lowercase()))
        .map(|(project, _)| *project)
        .collect();

    for (project, email) in &pairs {
        let lower = email.to_lowercase();
        if known.contains(&lower) {
            continue;
        }
        if is_noreply(email) {
            if insert_identity(conn, email, "noreply", now)? {
                report.noreply += 1;
            }
            link_to_primary(conn, email, config_emails, "local_part")?;
            continue;
        }
        let shares_local =
            local_part(email).is_some_and(|l| known_locals.contains(&l.to_lowercase()));
        if shares_local {
            if insert_identity(conn, email, "inferred", now)? {
                report.local_part += 1;
            }
            link_to_primary(conn, email, config_emails, "local_part")?;
            continue;
        }
        if known_projects.contains(project) {
            if insert_identity(conn, email, "inferred", now)? {
                report.coauthor += 1;
            }
            link_to_primary(conn, email, config_emails, "coauthor")?;
        }
    }

    Ok(report)
}

/// True when the row was new.
///
/// `_now` is threaded through because `seed` takes it and nothing here may read a clock; the
/// `identity` table has no timestamp column of its own until a confirmation stamps one.
fn insert_identity(
    conn: &rusqlite::Connection,
    email: &str,
    source: &str,
    _now: i64,
) -> Result<bool, IndexError> {
    let changed = conn.execute(
        "INSERT OR IGNORE INTO identity (email, name, is_user, source, confirmed_at)
         VALUES (?1, NULL, 1, ?2, NULL)",
        rusqlite::params![email, source],
    )?;
    Ok(changed == 1)
}

fn link_to_primary(
    conn: &rusqlite::Connection,
    email: &str,
    config_emails: &[String],
    reason: &str,
) -> Result<(), IndexError> {
    let primary = config_emails
        .iter()
        .find(
            |candidate| match (local_part(candidate), local_part(email)) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                _ => false,
            },
        )
        .or_else(|| config_emails.first());
    let Some(primary) = primary else {
        return Ok(());
    };
    let id: Option<i64> = conn
        .query_row(
            "SELECT id FROM identity WHERE email = ?1 COLLATE NOCASE",
            rusqlite::params![primary],
            |r| r.get(0),
        )
        .ok();
    let Some(id) = id else { return Ok(()) };
    conn.execute(
        "INSERT OR IGNORE INTO identity_alias (identity_id, email, reason) VALUES (?1, ?2, ?3)",
        rusqlite::params![id, email, reason],
    )?;
    Ok(())
}

/// Every seeded address, heaviest first.
///
/// # Errors
/// Returns [`IndexError`] when a read fails.
pub fn list(conn: &rusqlite::Connection) -> Result<Vec<IdentityRow>, IndexError> {
    let mut weights: BTreeMap<String, (i64, u32)> = BTreeMap::new();
    {
        let mut stmt = conn.prepare("SELECT email, commits FROM project_committer")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (email, commits) = row?;
            let entry = weights.entry(email.to_lowercase()).or_insert((0, 0));
            entry.0 = entry.0.saturating_add(commits);
            entry.1 = entry.1.saturating_add(1);
        }
    }

    let mut stmt = conn.prepare(
        "SELECT i.id, i.email, i.name, i.is_user, i.source, i.confirmed_at,
                a.reason, p.email
           FROM identity i
           LEFT JOIN identity_alias a ON a.email = i.email COLLATE NOCASE
           LEFT JOIN identity p ON p.id = a.identity_id
          ORDER BY i.id",
    )?;
    let mut out: Vec<IdentityRow> = stmt
        .query_map([], |r| {
            let email: String = r.get(1)?;
            let source: String = r.get(4)?;
            let reason: Option<String> = r.get(6)?;
            Ok(IdentityRow {
                id: IdentityId(r.get(0)?),
                email,
                name: r.get(2)?,
                is_user: r.get::<_, i64>(3)? != 0,
                source: match source.as_str() {
                    "gitconfig" => IdentitySource::Gitconfig,
                    "noreply" => IdentitySource::Noreply,
                    "manual" => IdentitySource::Manual,
                    _ => IdentitySource::Inferred,
                },
                alias_reason: reason.as_deref().map(|r| match r {
                    "coauthor" => AliasReason::Coauthor,
                    "manual" => AliasReason::Manual,
                    _ => AliasReason::LocalPart,
                }),
                primary_email: r.get(7)?,
                repositories: None,
                commits: 0,
                projects: 0,
                confirmed_at: r.get(5)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    for row in &mut out {
        if let Some((commits, projects)) = weights.get(&row.email.to_lowercase()) {
            row.commits = *commits;
            row.projects = *projects;
            row.repositories = Some(*projects);
        }
    }
    out.sort_by(|a, b| {
        b.commits
            .cmp(&a.commits)
            .then_with(|| a.email.cmp(&b.email))
    });
    Ok(out)
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
    use crate::protocol::{AliasReason, IdentitySource};

    const NOW: i64 = 1_760_000_000;

    fn open() -> (tempfile::TempDir, crate::index::Index) {
        let dir = tempfile::tempdir().unwrap();
        let index = crate::index::Index::open_at(dir.path(), NOW).unwrap();
        (dir, index)
    }

    fn project(conn: &rusqlite::Connection, name: &str) -> i64 {
        conn.execute(
            "INSERT INTO project (name, seed_basename, created_at, updated_at)
             VALUES (?1, ?1, ?2, ?2)",
            rusqlite::params![name, NOW],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn committer(conn: &rusqlite::Connection, project: i64, email: &str, commits: i64) {
        conn.execute(
            "INSERT INTO project_committer (project_id, email, commits) VALUES (?1, ?2, ?3)",
            rusqlite::params![project, email, commits],
        )
        .unwrap();
    }

    #[test]
    fn user_emails_are_read_out_of_a_config_and_nothing_else_is() {
        let text = concat!(
            "[user]\n\tname = A Person\n\temail = a@example.invalid\n",
            "[sendemail]\n\tfrom = noise@example.invalid\n",
            "[user]\n\temail = b@example.invalid\n",
        );
        assert_eq!(
            parse_config_emails(text),
            vec![
                "a@example.invalid".to_owned(),
                "b@example.invalid".to_owned()
            ]
        );
    }

    #[test]
    fn a_noreply_address_is_recognised_by_its_host() {
        assert!(is_noreply("1234+person@users.noreply.github.com"));
        assert!(!is_noreply("person@example.invalid"));
        assert_eq!(local_part("person+tag@example.invalid"), Some("person"));
        assert_eq!(local_part("broken"), None);
    }

    // §1.4: seeded from every user.email in configs, any noreply address, any address sharing a
    // local part with a known one, and any address co-authoring where a known identity also
    // authors.
    #[test]
    fn the_four_seeding_rules_each_produce_a_row() {
        let (_dir, index) = open();
        let conn = index.conn();
        let p1 = project(conn, "one");
        let p2 = project(conn, "two");
        committer(conn, p1, "a@example.invalid", 40);
        committer(conn, p1, "9+a@users.noreply.github.com", 3);
        committer(conn, p1, "a@other.invalid", 2);
        committer(conn, p2, "colleague@example.invalid", 11);
        committer(conn, p2, "a@example.invalid", 4);
        // An address that neither shares a local part nor ever sat beside a known identity.
        let p3 = project(conn, "three");
        committer(conn, p3, "stranger@example.invalid", 900);

        let report = seed(conn, &["a@example.invalid".to_owned()], NOW).unwrap();
        assert_eq!(report.from_config, 1);
        assert_eq!(report.noreply, 1);
        assert_eq!(report.local_part, 1);
        assert_eq!(report.coauthor, 1);

        let rows = list(conn).unwrap();
        let emails: Vec<&str> = rows.iter().map(|r| r.email.as_str()).collect();
        assert!(emails.contains(&"a@example.invalid"));
        assert!(emails.contains(&"9+a@users.noreply.github.com"));
        assert!(emails.contains(&"a@other.invalid"));
        assert!(emails.contains(&"colleague@example.invalid"));
        assert!(
            !emails.contains(&"stranger@example.invalid"),
            "the card narrows, never widens"
        );
    }

    #[test]
    fn seeding_twice_changes_nothing() {
        let (_dir, index) = open();
        let conn = index.conn();
        let p = project(conn, "one");
        committer(conn, p, "a@example.invalid", 5);
        seed(conn, &["a@example.invalid".to_owned()], NOW).unwrap();
        let first = list(conn).unwrap();
        seed(conn, &["a@example.invalid".to_owned()], NOW).unwrap();
        assert_eq!(list(conn).unwrap(), first);
    }

    // §1.4: the row's weight — <n> COMMITS IN <m> PROJECTS — read straight off
    // project_committer. Without it the row is unjudgeable.
    #[test]
    fn every_row_carries_its_weight_and_its_provenance() {
        let (_dir, index) = open();
        let conn = index.conn();
        let p1 = project(conn, "one");
        let p2 = project(conn, "two");
        committer(conn, p1, "a@example.invalid", 40);
        committer(conn, p2, "a@example.invalid", 2);
        committer(conn, p1, "a@other.invalid", 7);
        seed(conn, &["a@example.invalid".to_owned()], NOW).unwrap();

        let rows = list(conn).unwrap();
        let primary = rows
            .iter()
            .find(|r| r.email == "a@example.invalid")
            .unwrap();
        assert_eq!(primary.commits, 42);
        assert_eq!(primary.projects, 2);
        assert_eq!(primary.source, IdentitySource::Gitconfig);
        assert!(primary.is_user);
        assert_eq!(primary.confirmed_at, None, "seeded is not confirmed");

        let shared = rows.iter().find(|r| r.email == "a@other.invalid").unwrap();
        assert_eq!(shared.source, IdentitySource::Inferred);
        assert_eq!(shared.alias_reason, Some(AliasReason::LocalPart));
        assert_eq!(shared.primary_email.as_deref(), Some("a@example.invalid"));
    }

    // §1.4: an address seeded from a config that has authored nothing here is a real zero, and
    // it is worded rather than printed, because a `0` beside counts in the thousands reads as
    // a failed lookup. The wire carries the honest zero; the wording is the renderer's.
    #[test]
    fn a_config_address_with_no_commits_here_reports_a_real_zero() {
        let (_dir, index) = open();
        let conn = index.conn();
        seed(conn, &["a@example.invalid".to_owned()], NOW).unwrap();
        let rows = list(conn).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].commits, 0);
        assert_eq!(rows[0].projects, 0);
    }

    #[test]
    fn rows_come_back_heaviest_first_so_the_card_is_judgeable_at_a_glance() {
        let (_dir, index) = open();
        let conn = index.conn();
        let p = project(conn, "one");
        committer(conn, p, "a@example.invalid", 3);
        committer(conn, p, "a@other.invalid", 900);
        seed(conn, &["a@example.invalid".to_owned()], NOW).unwrap();
        let rows = list(conn).unwrap();
        assert_eq!(rows[0].email, "a@other.invalid");
    }
}

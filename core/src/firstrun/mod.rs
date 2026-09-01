//! First run: what is suggested, what is refused, and what the four screens are told.

pub mod classify;
pub mod refuse;
pub mod rescan;
pub mod roots;
pub mod sources;
pub mod suggest;

use std::sync::Arc;

use serde_json::Value;

use crate::index::path::PathPlatform;
use crate::index::IndexError;
use crate::proto::dispatch::{parse_args, CommandFailure}; // R15: one helper, plan 03's
use crate::scan::skiplist::SkipList;

/// The five commands of §10 this module answers.
pub const FIRST_RUN_COMMANDS: [&str; 5] = [
    "roots.suggest",
    "roots.add",
    "stats.reveal",
    "identity.list",
    "identity.confirm",
];

/// Everything first run needs that is not the database.
#[derive(Debug)]
pub struct FirstRunEnv {
    pub sources: sources::SourceEnv,
    pub classifier: Arc<dyn classify::RootClassifier>,
    pub distros: Arc<dyn classify::DistroProbe>,
    pub platform: PathPlatform,
    pub skip: SkipList,
    pub cache: roots::SuggestionCache,
}

/// Answer one of §10's commands, or decline it.
///
/// The core's `CommandHandler` tries this first and falls through on `None`; `None` means
/// *not mine*, and there is exactly one dispatcher in this module.
pub fn dispatch(
    conn: &mut rusqlite::Connection,
    env: &FirstRunEnv,
    command: &str,
    args: &Value,
    now: i64,
) -> Option<Result<Value, CommandFailure>> {
    match command {
        "roots.suggest" => Some(handle_suggest(env)),
        "roots.add" => Some(handle_add(conn, env, args, now)),
        "stats.reveal" => Some(handle_reveal(conn, now)),
        "identity.list" => Some(handle_identity_list(conn)),
        "identity.confirm" => Some(handle_identity_confirm(conn, args, now)),
        _ => None,
    }
}

fn internal(err: &IndexError) -> CommandFailure {
    CommandFailure::internal(err.to_string())
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Value, CommandFailure> {
    serde_json::to_value(value).map_err(|e| CommandFailure::internal(e.to_string()))
}

fn handle_suggest(env: &FirstRunEnv) -> Result<Value, CommandFailure> {
    let inputs = suggest::SuggestInputs {
        env: &env.sources,
        classifier: env.classifier.as_ref(),
        distros: env.distros.as_ref(),
        platform: env.platform,
    };
    let rows = suggest::suggest(&inputs);
    // What was suggested is what must not be estimated later (§10.1b).
    env.cache.remember(
        rows.iter()
            .map(|s| crate::paths::path_key(&s.path, env.platform)),
    );
    let wire: Vec<_> = rows.into_iter().map(|s| s.row).collect();
    encode(&wire)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddArgs {
    path_bytes: crate::protocol::Bytes,
    confirm_large: bool,
}

fn handle_add(
    conn: &mut rusqlite::Connection,
    env: &FirstRunEnv,
    args: &Value,
    now: i64,
) -> Result<Value, CommandFailure> {
    let parsed: AddArgs = parse_args(args.clone())?;
    let params = roots::AddParams {
        path_bytes: &parsed.path_bytes.0,
        confirm_large: parsed.confirm_large,
        home: &env.sources.home,
        skip: &env.skip,
        cache: &env.cache,
        platform: env.platform,
        distro: "",
        provenance: crate::protocol::RootProvenance::Dialog,
        now,
        ceiling_for_tests: refuse::DIRECTORY_CEILING,
    };
    let out = roots::add_root(conn, &params).map_err(|e| internal(&e))?;
    encode(&out)
}

fn handle_reveal(conn: &mut rusqlite::Connection, now: i64) -> Result<Value, CommandFailure> {
    let out = crate::stats::reveal::reveal(conn, now).map_err(|e| internal(&e))?;
    encode(&out)
}

fn handle_identity_list(conn: &mut rusqlite::Connection) -> Result<Value, CommandFailure> {
    let out = crate::identity::people::list(conn).map_err(|e| internal(&e))?;
    encode(&out)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfirmArgs {
    emails: Vec<String>,
    apply: bool,
}

fn handle_identity_confirm(
    conn: &mut rusqlite::Connection,
    args: &Value,
    now: i64,
) -> Result<Value, CommandFailure> {
    let parsed: ConfirmArgs = parse_args(args.clone())?;
    if parsed.apply {
        let out =
            crate::identity::confirm::apply(conn, &parsed.emails, now).map_err(|e| internal(&e))?;
        return encode(&out);
    }
    let delta =
        crate::identity::confirm::preview(conn, &parsed.emails).map_err(|e| internal(&e))?;
    encode(&crate::protocol::IdentityConfirm {
        moved_to_reference: delta.moved_to_reference,
        commit_days_removed: delta.commit_days_removed,
        applied: false,
    })
}

/// When first run ended, or `None` while it has not.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn first_run_completed_at(conn: &rusqlite::Connection) -> Result<Option<i64>, IndexError> {
    read_meta(conn, "first_run_completed_at")
}

/// The last completed scan, or `None`.
///
/// # Errors
/// Returns [`IndexError`] when the read fails.
pub fn last_scan_at(conn: &rusqlite::Connection) -> Result<Option<i64>, IndexError> {
    read_meta(conn, "last_scan_at")
}

/// One `app_meta` value as an integer, or `None` when the key is unset.
///
/// `optional()` rather than `.ok()`: rusqlite reports "no row" as an error, and swallowing
/// every error would read a corrupt or locked database as "first run never finished".
fn read_meta(conn: &rusqlite::Connection, key: &str) -> Result<Option<i64>, IndexError> {
    use rusqlite::OptionalExtension as _;
    let raw: Option<String> = conn
        .query_row(
            "SELECT v FROM app_meta WHERE k = ?1",
            rusqlite::params![key],
            |r| r.get(0),
        )
        .optional()?;
    Ok(raw.and_then(|v| v.parse().ok()))
}

/// Stamp the end of first run. Returns true when this call wrote it.
///
/// §11.3a: this is the residency answer's event, not the turn's button. Stamping at the turn
/// would begin the second-launch path with the question still unasked, and the row could never
/// reappear. **`settings.set` must call this whenever its patch carries `autostart`.**
///
/// # Errors
/// Returns [`IndexError`] when the write fails.
pub fn stamp_first_run_completed(
    conn: &rusqlite::Connection,
    now: i64,
) -> Result<bool, IndexError> {
    if first_run_completed_at(conn)?.is_some() {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO app_meta (k, v) VALUES ('first_run_completed_at', ?1)
           ON CONFLICT(k) DO NOTHING",
        rusqlite::params![now.to_string()],
    )?;
    Ok(true)
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
    use serde_json::json;

    const NOW: i64 = 1_760_000_000;

    fn open() -> (tempfile::TempDir, crate::index::Index) {
        let dir = tempfile::tempdir().unwrap();
        let index = crate::index::Index::open_at(dir.path(), NOW).unwrap();
        (dir, index)
    }

    fn env(home: &std::path::Path) -> FirstRunEnv {
        FirstRunEnv {
            sources: sources::SourceEnv {
                home: home.to_path_buf(),
                app_data: None,
                xdg_config: None,
            },
            classifier: std::sync::Arc::new(classify::FixedClassifier::new(vec![])),
            distros: std::sync::Arc::new(classify::NoDistros),
            platform: crate::index::path::PathPlatform::Unix,
            skip: crate::scan::skiplist::SkipList::default(),
            cache: roots::SuggestionCache::new(),
        }
    }

    #[test]
    fn the_shim_answers_every_command_it_declares_and_declines_the_rest() {
        let home = tempfile::tempdir().unwrap();
        let (_dir, mut index) = open();
        let e = env(home.path());
        // Written over the const, not over a literal count, so Task 17's four extra commands
        // extend the bar instead of leaving a test named after a number that has moved.
        for command in FIRST_RUN_COMMANDS {
            assert!(
                dispatch(index.conn_mut(), &e, command, &json!({}), NOW).is_some(),
                "{command} was declined"
            );
        }
        // `None` is "not mine" — plan 21's router chains the next dispatcher on it.
        assert!(dispatch(index.conn_mut(), &e, "projects.list", &json!({}), NOW).is_none());
    }

    #[test]
    fn suggesting_remembers_the_paths_so_the_estimate_stays_off_them() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("src")).unwrap();
        let (_dir, mut index) = open();
        let e = env(home.path());
        let out = dispatch(index.conn_mut(), &e, "roots.suggest", &json!({}), NOW)
            .unwrap()
            .unwrap();
        let rows = out.as_array().unwrap();
        assert!(!rows.is_empty());
        assert!(e.cache.contains(&crate::paths::path_key(
            &home.path().join("src"),
            crate::index::path::PathPlatform::Unix
        )));
    }

    #[test]
    fn a_malformed_argument_object_is_a_protocol_failure_and_not_a_panic() {
        let home = tempfile::tempdir().unwrap();
        let (_dir, mut index) = open();
        let e = env(home.path());
        let out = dispatch(index.conn_mut(), &e, "roots.add", &json!({"nope": 1}), NOW).unwrap();
        let failure = out.unwrap_err();
        assert_eq!(failure.code, crate::protocol::ErrorCode::Protocol);
    }

    // §1.4 / §2.4: apply:false writes nothing.
    #[test]
    fn identity_confirm_with_apply_false_reports_and_writes_nothing() {
        let home = tempfile::tempdir().unwrap();
        let (_dir, mut index) = open();
        let e = env(home.path());
        index
            .conn()
            .execute(
                "INSERT INTO identity (email, is_user, source)
                 VALUES ('a@example.invalid', 1, 'gitconfig')",
                [],
            )
            .unwrap();
        let out = dispatch(
            index.conn_mut(),
            &e,
            "identity.confirm",
            &json!({"emails": ["a@example.invalid"], "apply": false}),
            NOW,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            out.get("applied").and_then(serde_json::Value::as_bool),
            Some(false)
        );
        let confirmed: Option<i64> = index
            .conn()
            .query_row("SELECT confirmed_at FROM identity", [], |r| r.get(0))
            .unwrap();
        assert_eq!(confirmed, None);
    }

    // §11.3a: first_run_completed_at is stamped when the residency row is answered or
    // dismissed, not when the turn's button is pressed.
    #[test]
    fn the_first_run_stamp_is_written_once_and_never_moved() {
        let (_dir, index) = open();
        assert_eq!(first_run_completed_at(index.conn()).unwrap(), None);
        assert!(stamp_first_run_completed(index.conn(), NOW).unwrap());
        assert_eq!(first_run_completed_at(index.conn()).unwrap(), Some(NOW));
        assert!(!stamp_first_run_completed(index.conn(), NOW + 500).unwrap());
        assert_eq!(first_run_completed_at(index.conn()).unwrap(), Some(NOW));
    }
}

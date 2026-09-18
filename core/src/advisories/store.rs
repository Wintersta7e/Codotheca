//! Reads and writes for §32's nine tables.

use rusqlite::Connection;

use crate::index::IndexError;

/// The `x-ratelimit-resource` the sweep's own last **settled** response named, if one has.
///
/// **This is what the pre-issue budget read is keyed by**, and the reason is §32.3's second
/// defect. `DEFAULT_RESOURCE` is a process-wide constant read for every task before issuing. It is
/// right for the six authenticated REST reads phase 2 ships, but for this task it is an assumption:
/// nothing in this tree recorded which pool the advisories endpoint answers from.
///
/// If the endpoint names a resource other than `core`, [`crate::sync::budget::mirror`] writes
/// `(NULL, '<that name>')` while the pre-issue read looks up `(NULL, 'core')` — a pool the sweep
/// never writes. `may_spend` then answers `Unknown` for ever, and **`Unknown` spends**, so every
/// request issues with no brake at all until the source refuses. For an on-demand task that costs
/// one request and self-corrects; for a scheduled sweep it is the behaviour that gets the IP
/// rate-limited for every other unauthenticated call the app makes.
///
/// `None` until one response has named one, where the verdict is `Unknown`, which spends — one
/// request, self-correcting. **A response carrying no `x-ratelimit-resource` at all is not
/// mirrored and is not given a synthetic key here either**: with no resource header there is no
/// pool and no brake, and that is a property of the endpoint rather than a bug to work around.
///
/// # Errors
/// Fails when SQLite cannot be read.
pub fn last_settled_resource(conn: &Connection) -> Result<Option<String>, IndexError> {
    let found = conn
        .query_row(
            "SELECT resource FROM advisory_sweep
              WHERE settled_at IS NOT NULL AND resource IS NOT NULL
              ORDER BY settled_at DESC, id DESC
              LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();
    Ok(found)
}

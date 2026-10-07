//! §1.12's JSON sidecar: the five-plus categories a rebuild cannot re-derive.
//!
//! Every record is keyed on a `ProjectSubject`, never an id, because a rebuild reassigns ids
//! and an id-keyed restore puts one project's notes on another. Written atomically, hourly and
//! on clean shutdown, with a generation number and a checksum.
//!
//! Format 2 (§48.8) adds registered sections, every location's key and the pending records;
//! the reader still accepts format 1, which reads as format 2 with none of them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, Transaction};

use super::subject::subject_for_project;
use super::IndexError;
use crate::protocol::{LocationId, ProjectId};

/// The document shape this build writes. It also reads format 1 (§48.8.2).
pub const SIDECAR_FORMAT: u32 = 2;
/// How long after the last write, in seconds, a sidecar is due again: hourly (§1.12).
pub const SIDECAR_INTERVAL_SECS: i64 = 3600;

/// `app_meta` keys that describe the database or the sidecar's own bookkeeping.
///
/// The new database owns them, so they are neither exported nor restored — the restore skips
/// them too, so a key joining this list takes effect against a document written before it joined.
pub const REBUILD_OWNED_SETTINGS: [&str; 4] = [
    "schema_version",
    "sidecar_generation",
    "git_version",
    "sidecar_written_at",
];

/// `app_meta` keys no build reads any more, dropped on export and on restore.
///
/// This is the one place a retired key is dropped. `level_floor` is the legacy floor count, never
/// read as a level;
/// `restore_pending_generation` was written by a rebuild nothing called and read by nothing.
pub const RETIRED_SETTINGS: [&str; 2] = ["level_floor", "restore_pending_generation"];

/// Whether `key` stays out of the sidecar in both directions.
fn never_travels(key: &str) -> bool {
    REBUILD_OWNED_SETTINGS.contains(&key) || RETIRED_SETTINGS.contains(&key)
}

macro_rules! record {
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$field_meta:meta])* $field:ident : $ty:ty),* $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
        pub struct $name { $($(#[$field_meta])* pub $field: $ty),* }
    };
}

record!(
    /// A `location` row's natural key — what names a copy across a rebuild, which reassigns
    /// every location id.
    SidecarLocationKey {
        /// The side the copy is on: `win`, `linux` or `wsl`.
        kind: String,
        /// The WSL distribution, or empty for any other kind.
        distro: String,
        /// The location's `path_key` bytes, hex-encoded.
        path_key: String,
    }
);

/// One column's value, tagged with its SQLite storage class so a dumped row loads back as the
/// same value. A blob is hex, because a JSON string cannot carry arbitrary bytes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "t", content = "v")]
pub enum SidecarValue {
    /// SQL `NULL`.
    Null,
    /// An `INTEGER`.
    Integer(i64),
    /// A `REAL`.
    Real(f64),
    /// A `TEXT`.
    Text(String),
    /// A `BLOB`, hex-encoded.
    Blob(String),
}

// SQLite stores no NaN (it writes NULL instead) and JSON cannot carry one, so no value this type
// holds is unequal to itself.
impl Eq for SidecarValue {}

/// A whole table row, column name to value: what a record carries when no disk remains to
/// re-derive any column of it.
pub type SidecarRow = BTreeMap<String, SidecarValue>;

record!(
    /// One row of a registered section (§48.8.3): the subject it belongs to, the locations it
    /// names, and the section's own data.
    SectionRow {
        /// The subject key of the project the row belongs to; `None` for a row of no project.
        subject: Option<String>,
        /// The keys of every location the row references, resolved again on restore.
        location_keys: Vec<SidecarLocationKey>,
        /// The section's data, in the shape its owner's export writes and its restore reads.
        data: serde_json::Value,
    }
);
record!(
    /// One `sidecar_pending` row, carried verbatim (§48.8.4): a record staged by a rebuild and
    /// not yet matched to a project. Its id stays behind — it is local to one database.
    PendingRow {
        /// The generation of the sidecar the record was staged from.
        source_generation: i64,
        /// The subject key the record waits for.
        subject_key: String,
        /// The JSON text of the record's location keys, as the column holds it.
        location_keys: String,
        /// The JSON text of the record itself, as the column holds it.
        record: String,
        /// When it was staged, in Unix seconds.
        queued_at: i64,
    }
);
/// When a section's rows come back after a rebuild (§48.8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// In the rebuild transaction, before any scan.
    Global,
    /// When the scan rediscovers the row's subject, through the pending table (§48.8.4).
    Subject,
    /// In the rebuild transaction, for a project no scan can ever rediscover.
    NoScan,
}

/// How a restored value meets one already present (§48.8.3): never lowering an earned value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreRule {
    /// The restored row is written as it was exported.
    Replace,
    /// The value restores as `max(present, restored)`.
    RaiseOnly,
    /// The value restores only if none is present.
    WriteOnce,
    /// The latch restores only if it is not already set.
    Latch,
}

/// What one section restore did with one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// The row was written: this many rows went into the index.
    Applied(u64),
    /// Nothing was written, because a location the row names resolves to no location yet; the
    /// matcher keeps the pending record for the hand-off that brings the location back. Only a
    /// `Subject` restore may answer this.
    Pending,
}

/// What a section restore is handed besides its row.
#[derive(Debug, Clone, Copy)]
pub struct RestoreCtx<'a> {
    /// The restore's time, in Unix seconds.
    pub now: i64,
    /// The project the row was matched to; `None` for a `Global` or `NoScan` row of no project.
    pub project: Option<ProjectId>,
    /// Each of the row's location keys resolved to the location it names now, after removed
    /// copies were re-created; a key missing here resolved to nothing.
    pub locations: &'a [(SidecarLocationKey, LocationId)],
    /// The generation of the sidecar the row came from.
    pub source_generation: u64,
}

impl RestoreCtx<'_> {
    /// The location `key` names now, or `None` when it resolved to none.
    #[must_use]
    pub fn location_for(&self, key: &SidecarLocationKey) -> Option<LocationId> {
        self.locations
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, id)| *id)
    }
}

/// One registered section: a non-derivable table, its owner, and how it travels.
#[derive(Debug, Clone, Copy)]
pub struct SectionSpec {
    /// The section's key in the document's `sections` map.
    pub name: &'static str,
    /// The spec section that owns the data, as `§NN`.
    pub owner: &'static str,
    /// When its rows come back.
    pub scope: Scope,
    /// How a restored value meets a present one.
    pub rule: RestoreRule,
    /// Every `AUTOINCREMENT` table whose ids name something outside the index; the rebuild
    /// raises each one's sequence past the highest id restored.
    pub preserves_ids: &'static [&'static str],
    /// Reads every row the section carries.
    pub export: fn(&Connection) -> Result<Vec<SectionRow>, IndexError>,
    /// Writes one row back through the owner's one writer.
    pub restore:
        fn(&Transaction<'_>, &SectionRow, &RestoreCtx<'_>) -> Result<RestoreOutcome, IndexError>,
}

/// The one section registry, which the export, the reader and the rebuild all read.
///
/// A change that lands a table, a `project`/`location` column, or an `app_meta` key does all of
/// the following in that same change:
///
/// 1. **Classify every new table and every new `project`/`location` column** in the registry's
///    classification list: derivable, naming the producer that re-derives it, or a section. The
///    registration gate fails on anything unclassified.
/// 2. **A non-derivable table registers a [`SectionSpec`]** here: `name`, `owner` (`§NN`),
///    `scope` (`Global | Subject | NoScan`), `rule` (`Replace | RaiseOnly | WriteOnce | Latch`),
///    `preserves_ids` (every `AUTOINCREMENT` table whose ids name something outside the index),
///    `export`, `restore`. **`restore` calls the owner's one writer** (§48.8.3); a raise-only
///    value restores as `max(present, restored)`, a latch and a write-once value only if absent.
///    **`restore` answers `Result<RestoreOutcome, IndexError>`**: `Applied(n)` once written, or
///    `Pending` — having written nothing — when a row's location key resolves to no location;
///    **the matcher deletes only an `Applied` record**, so a `Pending` one waits for the scan that
///    brings the location back (§48.8.4). Only a `Subject` restore may answer `Pending`. **Keep
///    your typed export/restore functions; register adapters with `SectionSpec`'s exact
///    signatures**, declared beside the typed functions, which they call.
/// 3. **Subject rows carry `location_keys`** whenever they reference a `location`: the matcher
///    hands `restore` a resolved `LocationId` per key, after re-creating removed locations; a key
///    [`RestoreCtx::location_for`] cannot resolve is the restore's `Pending`.
/// 4. **A field added to a format-1 record** is declared
///    `#[serde(default, skip_serializing_if = "Option::is_none")]`, so a format-1 document's
///    checksum still verifies. **No bump.** The format number is 2 for the rest of phase 4.
/// 5. **An `app_meta` key** rides `settings` unless it describes the database or the sidecar's
///    own bookkeeping; then it joins [`REBUILD_OWNED_SETTINGS`].
/// 6. **Add the section's noun** to `SIDECAR_COUNT_NOUNS` in
///    `app/src/renderer/failure/copy.ts`; the mirror test reads the section names from this file.
/// 7. **Add a fixture** to `testing::sidecar::SECTION_FIXTURES`; the registry test asserts every
///    registered section has one.
/// 8. **`rule` other than `Replace`** adds its case to
///    `core/tests/sidecar_restore_never_lowers.rs`.
/// 9. **An act that orphans bytes outside the index** (§48.8.6) adds an `ExportAct` variant,
///    calls `export_after_act` after its commit, and adds its case to
///    `core/tests/sidecar_act_exports.rs`.
/// 10. **A write that computes or changes a subject key** calls `pending::match_pending` in its
///     transaction. **A restore or re-resolution that maps a subject to a project calls
///     `pending::resolve_subject_unique`** — `Some` only when exactly one live project holds it —
///     never `subject::resolve_subject`, which answers the lowest id.
pub const SECTIONS: &[SectionSpec] = &[SectionSpec {
    name: "no_scan_projects",
    owner: "§48",
    scope: Scope::NoScan,
    rule: RestoreRule::Replace,
    preserves_ids: &["project", "location"],
    export: super::noscan::export_rows,
    restore: super::noscan::restore_row,
}];

/// The count keys every document has, before one key per registered section.
pub const BASE_COUNT_KEYS: [&str; 14] = [
    "projects",
    "notes",
    "sessions",
    "session_segments",
    "xp_events",
    "launch_targets",
    "collections",
    "collection_members",
    "roots",
    "identities",
    "merges",
    "settings",
    "view_state",
    "pending",
];

record!(
    /// One `session_segment` row.
    SidecarSegment {
        /// When the segment began, in Unix seconds.
        started_at: i64,
        /// When it ended, in Unix seconds; `None` while it was still open.
        ended_at: Option<i64>,
        /// The seconds it credited to its session (§9).
        credited_seconds: i64,
        /// What ended it — `idle`, `session_end`, `app_exit` or `crash`; `None` exactly when
        /// `ended_at` is.
        closed_by: Option<String>,
    }
);
record!(
    /// One `session` row, with its segments.
    SidecarSession {
        /// When the session began, in Unix seconds.
        started_at: i64,
        /// When it ended, in Unix seconds; `None` while it was still open.
        ended_at: Option<i64>,
        /// The session's credit — always the sum of its segments' (§9).
        credited_seconds: i64,
        /// What ended it — `stop`, `idle`, `process_exit`, `app_exit`, `crash` or `orphaned`;
        /// `None` exactly when `ended_at` is.
        close_reason: Option<String>,
        /// Its segments, oldest first.
        segments: Vec<SidecarSegment>,
        /// The copy it ran in; `None` in format 1 and for a session that recorded none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        location_key: Option<SidecarLocationKey>,
    }
);
record!(
    /// One session-track `xp_events` row. The git track is not exported: §1.7 makes it a pure
    /// function of history, which a rebuild re-derives.
    SidecarXpEvent {
        /// When the event happened, in Unix seconds.
        ts: i64,
        /// The machine's offset from UTC at the time, in minutes east; `None` when not recorded.
        tz_offset_min: Option<i64>,
        /// The event kind — on this track `session`, `focus` or `debt_day`.
        kind: String,
        /// The row's unique key; restoring a key that already exists writes nothing.
        dedupe_key: String,
        /// The kind's JSON detail, if it carries any.
        meta: Option<String>,
    }
);
record!(
    /// One launch target the user made. Detected targets are not exported; they are re-detected.
    SidecarLaunchTarget {
        /// What it opens the project in — `editor`, `terminal`, `file_manager` or `git_client`.
        kind: String,
        /// Its display name.
        name: String,
        /// The executable path's bytes, hex-encoded, because a path is bytes and not text.
        exec_hex: String,
        /// Its argument list, as the JSON text `launch_target.args_json` holds.
        args_json: String,
        /// `location` to start it in the project's copy, `none` to set no working directory.
        cwd_mode: String,
        /// Its extra environment, as the JSON text `launch_target.env_json` holds.
        env_json: String,
        /// Its position within its scope; the lowest is the default.
        sort_index: i64,
        /// The language it applies to; `None` means any language, never an unknown one.
        language: Option<String>,
    }
);
record!(
    /// One unmerged project's non-derivable state, keyed on its subject.
    SidecarProject {
        /// The project's [`ProjectSubject`] key — never an id, which a rebuild reassigns.
        ///
        /// [`ProjectSubject`]: super::subject::ProjectSubject
        subject: String,
        /// The user's note on the project, if any.
        notes: Option<String>,
        /// Whether the user pinned it.
        is_pinned: bool,
        /// Whether the user archived it.
        is_archived: bool,
        /// Whether the user hid it.
        is_hidden: bool,
        /// When it was first opened or launched, in Unix seconds (§10.5); `None` if never.
        acknowledged_at: Option<i64>,
        /// The directory basename at first index, which every art value derives from (§7.4).
        seed_basename: String,
        /// How many times `art.rerender` re-rolled its art (§7.2).
        reroll_offset: i64,
        /// Its sessions, oldest first.
        sessions: Vec<SidecarSession>,
        /// Its session-track XP rows, oldest first.
        xp_events: Vec<SidecarXpEvent>,
        /// Its user-made launch targets, in `sort_index` order.
        launch_targets: Vec<SidecarLaunchTarget>,
        /// The key of every location the project has, removed ones included; empty in format 1.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        location_keys: Vec<SidecarLocationKey>,
        /// Every column of each removed location, which no scan can re-derive; empty in format 1.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        removed_locations: Vec<SidecarRow>,
    }
);
record!(
    /// One collection definition, with its members as subject keys.
    SidecarCollection {
        /// The collection's name, unique regardless of case.
        name: String,
        /// `manual` for a hand-picked list, `query` for a saved query.
        kind: String,
        /// The query a `query` collection runs; `None` for a `manual` one.
        query_text: Option<String>,
        /// The grammar version `query_text` is written in; `None` for a `manual` collection.
        query_grammar_version: Option<i64>,
        /// Its position in the collection list.
        sort_index: i64,
        /// Subject keys of its member projects; a member with no subject is left out.
        members: Vec<String>,
    }
);
record!(
    /// One `scan_root` row.
    SidecarRoot {
        /// The side the root is on: `win`, `linux` or `wsl`.
        kind: String,
        /// The WSL distribution, or empty for any other kind.
        distro: String,
        /// The root's path bytes, hex-encoded; its `path_key` is recomputed on restore.
        path_hex: String,
        /// Whether the scan walks it.
        enabled: bool,
        /// Who added it: `suggested` or `user`.
        added_by: String,
        /// Whether the walk continues below a repository root (§4.2).
        descend_into_repos: bool,
        /// When it was added, in Unix seconds.
        added_at: i64,
    }
);
record!(
    /// One further address recognised as the same identity (§1.4).
    SidecarAlias {
        /// The alias address.
        email: String,
        /// Why it is an alias: `local_part`, `coauthor` or `manual`.
        reason: String,
    }
);
record!(
    /// One `identity` row, with its aliases (§1.4).
    SidecarIdentity {
        /// The address, unique regardless of case.
        email: String,
        /// The name recorded with it, if any.
        name: Option<String>,
        /// Whether the address is the user's own rather than one recorded for someone else.
        is_user: bool,
        /// Where the address came from: `gitconfig`, `noreply`, `inferred` or `manual`.
        source: String,
        /// When the user confirmed the set, in Unix seconds; `None` is seeded and never confirmed.
        confirmed_at: Option<i64>,
        /// Its aliases.
        aliases: Vec<SidecarAlias>,
    }
);
record!(
    /// One `merge_record` row, both sides keyed on their subjects (§1.9).
    SidecarMerge {
        /// Subject key of the project that absorbed the other.
        survivor: String,
        /// Subject key of the project merged into it.
        absorbed: String,
        /// When the merge happened, in Unix seconds.
        merged_at: i64,
        /// How the two were associated: `definitive`, `strong`, `inferred` or `manual`.
        association_kind: String,
        /// The evidence the merge was decided on, as JSON text.
        evidence_json: String,
        /// The absorbed row as it stood before the merge, as JSON text — what a split restores.
        absorbed_json: String,
    }
);

/// Everything a sidecar carries: the non-derivable set, with no row keyed on an id.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SidecarPayload {
    /// Every unmerged project that has a subject, with what hangs off it.
    pub projects: Vec<SidecarProject>,
    /// Every collection.
    pub collections: Vec<SidecarCollection>,
    /// Every scan root.
    pub roots: Vec<SidecarRoot>,
    /// Every identity, with its aliases.
    pub identities: Vec<SidecarIdentity>,
    /// Every merge record whose two sides both have a subject.
    pub merges: Vec<SidecarMerge>,
    /// `app_meta`, less the keys the new database owns.
    pub settings: BTreeMap<String, String>,
    /// Every `view_state` row.
    pub view_state: BTreeMap<String, String>,
    /// Each registered section's rows, by section name; empty in format 1.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sections: BTreeMap<String, Vec<SectionRow>>,
    /// Every unconsumed `sidecar_pending` row, so a second corruption loses none of the first's;
    /// empty in format 1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<PendingRow>,
}

/// The sidecar document as written to `index-sidecar.json`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sidecar {
    /// The document shape: 1 or [`SIDECAR_FORMAT`]; [`inspect`] calls any other newer.
    pub format: u32,
    /// Which write this is; each export adds one to `app_meta.sidecar_generation`.
    pub generation: u64,
    /// When it was written, in Unix seconds — where a rebuild's gap starts.
    pub written_at: i64,
    /// The writing index's `user_version`; `None` in format 1. Outside the payload, so the
    /// checksum does not cover it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    /// `fnv1a64:<hex>` over the serialised payload; [`inspect`] calls a mismatch unreadable.
    pub checksum: String,
    /// The records themselves.
    pub payload: SidecarPayload,
}

/// What [`Index::export_sidecar`] wrote.
///
/// [`Index::export_sidecar`]: super::Index::export_sidecar
#[derive(Debug, Clone)]
pub struct SidecarWriteReport {
    /// Where the sidecar was written.
    pub path: PathBuf,
    /// The generation it carries.
    pub generation: u64,
    /// When it was written, in Unix seconds.
    pub written_at: i64,
    /// How many of each record it holds, by [`counts`]' keys.
    pub counts: BTreeMap<String, u64>,
}

/// Whether a sidecar is due: none was ever written, or [`SIDECAR_INTERVAL_SECS`] have passed.
#[must_use]
pub const fn is_due(last_written_at: Option<i64>, now: i64) -> bool {
    match last_written_at {
        None => true,
        Some(last) => now.saturating_sub(last) >= SIDECAR_INTERVAL_SECS,
    }
}

/// An act that changes what exists outside the index (§48.8.6).
///
/// A rebuild restores from the last sidecar, so one written before such an act brings back a
/// library the disk no longer matches; each act therefore exports through [`export_after_act`]
/// once it has committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportAct {
    /// `locations.uninstall`: a copy's files went to the bin and its row was marked removed.
    Uninstall,
}

impl ExportAct {
    /// Every act, for the test that drives each one through its production arm.
    pub const ALL: &'static [Self] = &[Self::Uninstall];
}

/// Export the sidecar after `act` has committed — never inside its transaction, so an export
/// that fails cannot roll the act back.
///
/// # Errors
/// [`Index::export_sidecar`]'s: SQLite refuses a read or a write, or the file cannot be written
/// and renamed into place.
///
/// [`Index::export_sidecar`]: super::Index::export_sidecar
pub fn export_after_act(
    index: &super::Index,
    act: ExportAct,
    now: i64,
) -> Result<SidecarWriteReport, IndexError> {
    match act {
        ExportAct::Uninstall => index.export_sidecar(now),
    }
}

fn len_u64<T>(v: &[T]) -> u64 {
    u64::try_from(v.len()).unwrap_or(u64::MAX)
}

/// Count each kind of record `s` holds: every [`BASE_COUNT_KEYS`] entry, then one key per section
/// the document carries.
#[must_use]
pub fn counts(s: &Sidecar) -> BTreeMap<String, u64> {
    let p = &s.payload;
    let (mut notes, mut sessions, mut segments, mut xp_events, mut targets) = (0, 0, 0, 0, 0);
    for project in &p.projects {
        if project.notes.is_some() {
            notes += 1;
        }
        sessions += len_u64(&project.sessions);
        for session in &project.sessions {
            segments += len_u64(&session.segments);
        }
        xp_events += len_u64(&project.xp_events);
        targets += len_u64(&project.launch_targets);
    }
    let members = p.collections.iter().map(|c| len_u64(&c.members)).sum();
    let base: [(&str, u64); 14] = [
        ("projects", len_u64(&p.projects)),
        ("notes", notes),
        ("sessions", sessions),
        ("session_segments", segments),
        ("xp_events", xp_events),
        ("launch_targets", targets),
        ("collections", len_u64(&p.collections)),
        ("collection_members", members),
        ("roots", len_u64(&p.roots)),
        ("identities", len_u64(&p.identities)),
        ("merges", len_u64(&p.merges)),
        (
            "settings",
            u64::try_from(p.settings.len()).unwrap_or(u64::MAX),
        ),
        (
            "view_state",
            u64::try_from(p.view_state.len()).unwrap_or(u64::MAX),
        ),
        ("pending", len_u64(&p.pending)),
    ];
    let mut out: BTreeMap<String, u64> = base
        .into_iter()
        .map(|(key, n)| (key.to_owned(), n))
        .collect();
    for (name, rows) in &p.sections {
        out.insert(name.clone(), len_u64(rows));
    }
    out
}

/// Read the non-derivable set out of the database as a checksummed sidecar document.
///
/// # Errors
/// Fails when SQLite refuses a read, or the payload does not serialise for its checksum.
pub fn export(conn: &Connection, generation: u64, now: i64) -> Result<Sidecar, IndexError> {
    let payload = SidecarPayload {
        projects: export_projects(conn)?,
        collections: export_collections(conn)?,
        roots: export_roots(conn)?,
        identities: export_identities(conn)?,
        merges: export_merges(conn)?,
        settings: export_kv(conn, "app_meta", never_travels)?,
        view_state: export_kv(conn, "view_state", |_| false)?,
        sections: export_sections(conn)?,
        pending: export_pending(conn)?,
    };
    let checksum = checksum_of(&payload)?;
    Ok(Sidecar {
        format: SIDECAR_FORMAT,
        generation,
        written_at: now,
        schema_version: Some(super::migrate::schema_version(conn)?),
        checksum,
        payload,
    })
}

/// FNV-1a/64 over the serialised payload. Deterministic because the payload is structs and
/// `BTreeMap`s, never `HashMap`s.
fn checksum_of(payload: &SidecarPayload) -> Result<String, IndexError> {
    let bytes = serde_json::to_vec(payload).map_err(|e| IndexError::Sidecar(e.to_string()))?;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in &bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Ok(format!("fnv1a64:{h:016x}"))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn export_kv(
    conn: &Connection,
    table: &str,
    skip: fn(&str) -> bool,
) -> Result<BTreeMap<String, String>, IndexError> {
    let mut out = BTreeMap::new();
    let mut stmt = conn.prepare(&format!("SELECT k, v FROM {table} ORDER BY k"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let k: String = row.get(0)?;
        if skip(&k) {
            continue;
        }
        out.insert(k, row.get(1)?);
    }
    Ok(out)
}

fn export_projects(conn: &Connection) -> Result<Vec<SidecarProject>, IndexError> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, notes, is_pinned, is_archived, is_hidden, acknowledged_at,
                seed_basename, reroll_offset
         FROM project WHERE merged_into IS NULL ORDER BY id",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let id = ProjectId(row.get(0)?);
        let Some(subject) = subject_for_project(conn, id)? else {
            // No lineage and no location: nothing outlives the rebuild to key on.
            continue;
        };
        out.push(SidecarProject {
            subject: subject.to_key(),
            notes: row.get(1)?,
            is_pinned: row.get::<_, i64>(2)? != 0,
            is_archived: row.get::<_, i64>(3)? != 0,
            is_hidden: row.get::<_, i64>(4)? != 0,
            acknowledged_at: row.get(5)?,
            seed_basename: row.get(6)?,
            reroll_offset: row.get(7)?,
            sessions: export_sessions(conn, id)?,
            xp_events: export_xp_events(conn, id)?,
            launch_targets: export_targets(conn, id)?,
            location_keys: export_location_keys(conn, id)?,
            removed_locations: export_removed_locations(conn, id)?,
        });
    }
    Ok(out)
}

/// Every location's key, removed ones included: what a later record names a copy by.
fn export_location_keys(
    conn: &Connection,
    id: ProjectId,
) -> Result<Vec<SidecarLocationKey>, IndexError> {
    let mut stmt = conn
        .prepare("SELECT kind, distro, path_key FROM location WHERE project_id = ?1 ORDER BY id")?;
    let mut rows = stmt.query([id.0])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let path_key: Vec<u8> = r.get(2)?;
        out.push(SidecarLocationKey {
            kind: r.get(0)?,
            distro: r.get(1)?,
            path_key: hex(&path_key),
        });
    }
    Ok(out)
}

/// Every column of each location this app removed — no disk remains to re-derive one.
fn export_removed_locations(
    conn: &Connection,
    id: ProjectId,
) -> Result<Vec<SidecarRow>, IndexError> {
    let ids: Vec<i64> = conn
        .prepare(
            "SELECT id FROM location WHERE project_id = ?1 AND removed_at IS NOT NULL ORDER BY id",
        )?
        .query_map([id.0], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    ids.into_iter()
        .map(|location| dump_row(conn, "location", location))
        .collect()
}

/// The key of a dumped `location` row: its `kind`, `distro` and `path_key`.
///
/// # Errors
/// Fails when the row lacks one of the three, or holds one in another storage class.
pub(crate) fn location_key_of_row(row: &SidecarRow) -> Result<SidecarLocationKey, IndexError> {
    let text = |column: &str| match row.get(column) {
        Some(SidecarValue::Text(value)) => Ok(value.clone()),
        _ => Err(IndexError::Sidecar(format!(
            "a location row carries no {column}"
        ))),
    };
    let Some(SidecarValue::Blob(path_key)) = row.get("path_key") else {
        return Err(IndexError::Sidecar(
            "a location row carries no path_key".to_owned(),
        ));
    };
    Ok(SidecarLocationKey {
        kind: text("kind")?,
        distro: text("distro")?,
        path_key: path_key.clone(),
    })
}

/// The key of the location `id` names, or `None` when it names none.
fn location_key(
    conn: &Connection,
    id: Option<i64>,
) -> Result<Option<SidecarLocationKey>, IndexError> {
    use rusqlite::OptionalExtension as _;
    let Some(id) = id else {
        return Ok(None);
    };
    let row: Option<(String, String, Vec<u8>)> = conn
        .query_row(
            "SELECT kind, distro, path_key FROM location WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    Ok(row.map(|(kind, distro, path_key)| SidecarLocationKey {
        kind,
        distro,
        path_key: hex(&path_key),
    }))
}

/// Each registered section's rows, under its name.
fn export_sections(conn: &Connection) -> Result<BTreeMap<String, Vec<SectionRow>>, IndexError> {
    let mut out = BTreeMap::new();
    for section in SECTIONS {
        out.insert(section.name.to_owned(), (section.export)(conn)?);
    }
    Ok(out)
}

/// Every `sidecar_pending` row, verbatim, oldest generation first.
fn export_pending(conn: &Connection) -> Result<Vec<PendingRow>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT source_generation, subject_key, location_keys, record, queued_at
         FROM sidecar_pending ORDER BY source_generation, id",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(PendingRow {
                source_generation: r.get(0)?,
                subject_key: r.get(1)?,
                location_keys: r.get(2)?,
                record: r.get(3)?,
                queued_at: r.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn export_sessions(conn: &Connection, id: ProjectId) -> Result<Vec<SidecarSession>, IndexError> {
    let mut sessions = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, started_at, ended_at, credited_seconds, close_reason, location_id
         FROM session WHERE project_id = ?1 ORDER BY started_at, id",
    )?;
    let mut rows = stmt.query([id.0])?;
    while let Some(row) = rows.next()? {
        let session_id: i64 = row.get(0)?;
        let mut seg_stmt = conn.prepare(
            "SELECT started_at, ended_at, credited_seconds, closed_by
             FROM session_segment WHERE session_id = ?1 ORDER BY started_at, id",
        )?;
        let mut seg_rows = seg_stmt.query([session_id])?;
        let mut segments = Vec::new();
        while let Some(s) = seg_rows.next()? {
            segments.push(SidecarSegment {
                started_at: s.get(0)?,
                ended_at: s.get(1)?,
                credited_seconds: s.get(2)?,
                closed_by: s.get(3)?,
            });
        }
        sessions.push(SidecarSession {
            started_at: row.get(1)?,
            ended_at: row.get(2)?,
            credited_seconds: row.get(3)?,
            close_reason: row.get(4)?,
            segments,
            location_key: location_key(conn, row.get(5)?)?,
        });
    }
    Ok(sessions)
}

fn export_xp_events(conn: &Connection, id: ProjectId) -> Result<Vec<SidecarXpEvent>, IndexError> {
    // The session track only: §1.7 makes the git track a pure function of history, which a
    // rebuild re-derives.
    let mut stmt = conn.prepare(
        "SELECT ts, tz_offset_min, kind, dedupe_key, meta
         FROM xp_events WHERE project_id = ?1 AND track = 'session' ORDER BY ts, id",
    )?;
    let mut rows = stmt.query([id.0])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(SidecarXpEvent {
            ts: r.get(0)?,
            tz_offset_min: r.get(1)?,
            kind: r.get(2)?,
            dedupe_key: r.get(3)?,
            meta: r.get(4)?,
        });
    }
    Ok(out)
}

fn export_targets(
    conn: &Connection,
    id: ProjectId,
) -> Result<Vec<SidecarLaunchTarget>, IndexError> {
    // Custom rows only: a detected row is re-detected, and §4bis rewrites its exec_bytes anyway.
    let mut stmt = conn.prepare(
        "SELECT kind, name, exec_bytes, args_json, cwd_mode, env_json, sort_index, language
         FROM launch_target WHERE project_id = ?1 AND detected = 0 ORDER BY sort_index, id",
    )?;
    let mut rows = stmt.query([id.0])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let exec: Vec<u8> = r.get(2)?;
        out.push(SidecarLaunchTarget {
            kind: r.get(0)?,
            name: r.get(1)?,
            exec_hex: hex(&exec),
            args_json: r.get(3)?,
            cwd_mode: r.get(4)?,
            env_json: r.get(5)?,
            sort_index: r.get(6)?,
            language: r.get(7)?,
        });
    }
    Ok(out)
}

fn export_collections(conn: &Connection) -> Result<Vec<SidecarCollection>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, kind, query_text, query_grammar_version, sort_index
         FROM collection ORDER BY sort_index, id",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let cid: i64 = r.get(0)?;
        let mut m_stmt = conn.prepare(
            "SELECT project_id FROM collection_member WHERE collection_id = ?1 ORDER BY project_id",
        )?;
        let mut m_rows = m_stmt.query([cid])?;
        let mut members = Vec::new();
        while let Some(m) = m_rows.next()? {
            if let Some(s) = subject_for_project(conn, ProjectId(m.get(0)?))? {
                members.push(s.to_key());
            }
        }
        out.push(SidecarCollection {
            name: r.get(1)?,
            kind: r.get(2)?,
            query_text: r.get(3)?,
            query_grammar_version: r.get(4)?,
            sort_index: r.get(5)?,
            members,
        });
    }
    Ok(out)
}

fn export_roots(conn: &Connection) -> Result<Vec<SidecarRoot>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT kind, distro, path_bytes, enabled, added_by, descend_into_repos, added_at
         FROM scan_root ORDER BY id",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let bytes: Vec<u8> = r.get(2)?;
        out.push(SidecarRoot {
            kind: r.get(0)?,
            distro: r.get(1)?,
            path_hex: hex(&bytes),
            enabled: r.get::<_, i64>(3)? != 0,
            added_by: r.get(4)?,
            descend_into_repos: r.get::<_, i64>(5)? != 0,
            added_at: r.get(6)?,
        });
    }
    Ok(out)
}

fn export_identities(conn: &Connection) -> Result<Vec<SidecarIdentity>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT id, email, name, is_user, source, confirmed_at FROM identity ORDER BY id",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let iid: i64 = r.get(0)?;
        let mut a_stmt = conn.prepare(
            "SELECT email, reason FROM identity_alias WHERE identity_id = ?1 ORDER BY id",
        )?;
        let mut a_rows = a_stmt.query([iid])?;
        let mut aliases = Vec::new();
        while let Some(a) = a_rows.next()? {
            aliases.push(SidecarAlias {
                email: a.get(0)?,
                reason: a.get(1)?,
            });
        }
        out.push(SidecarIdentity {
            email: r.get(1)?,
            name: r.get(2)?,
            is_user: r.get::<_, i64>(3)? != 0,
            source: r.get(4)?,
            confirmed_at: r.get(5)?,
            aliases,
        });
    }
    Ok(out)
}

fn export_merges(conn: &Connection) -> Result<Vec<SidecarMerge>, IndexError> {
    let mut stmt = conn.prepare(
        "SELECT survivor_project_id, absorbed_project_id, merged_at, association_kind,
                evidence_json, absorbed_json
         FROM merge_record ORDER BY id",
    )?;
    let mut rows = stmt.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        let survivor = subject_for_project(conn, ProjectId(r.get(0)?))?;
        let absorbed = subject_for_project(conn, ProjectId(r.get(1)?))?;
        let (Some(survivor), Some(absorbed)) = (survivor, absorbed) else {
            continue;
        };
        out.push(SidecarMerge {
            survivor: survivor.to_key(),
            absorbed: absorbed.to_key(),
            merged_at: r.get(2)?,
            association_kind: r.get(3)?,
            evidence_json: r.get(4)?,
            absorbed_json: r.get(5)?,
        });
    }
    Ok(out)
}

/// Every column of one row of `table`, read by `rowid`: the one dumper, so a record that must
/// carry a whole row carries every column the table has, including one added after this code.
///
/// # Errors
/// Fails when SQLite refuses the read, no row has that `rowid`, or a `TEXT` value is not UTF-8.
pub fn dump_row(conn: &Connection, table: &str, rowid: i64) -> Result<SidecarRow, IndexError> {
    let mut stmt = conn.prepare(&format!("SELECT * FROM {table} WHERE rowid = ?1"))?;
    let names: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
    let row = stmt.query_row([rowid], |r| {
        let mut row = SidecarRow::new();
        for (i, name) in names.iter().enumerate() {
            let value = match r.get_ref(i)? {
                ValueRef::Null => SidecarValue::Null,
                ValueRef::Integer(n) => SidecarValue::Integer(n),
                ValueRef::Real(x) => SidecarValue::Real(x),
                ValueRef::Text(_) => SidecarValue::Text(r.get(i)?),
                ValueRef::Blob(bytes) => SidecarValue::Blob(hex(bytes)),
            };
            row.insert(name.clone(), value);
        }
        Ok(row)
    })?;
    Ok(row)
}

/// Insert a dumped row into `table`: the one loader.
///
/// Names only the columns `table` has and `skip` does not list, so a column the row carries and
/// the table lost is dropped rather than refused, and a skipped `id` takes the table's next.
///
/// # Errors
/// Fails when SQLite refuses the column read or the insert, or a blob's hex is malformed.
pub fn insert_row(
    tx: &Transaction<'_>,
    table: &str,
    row: &SidecarRow,
    skip: &[&str],
) -> Result<(), IndexError> {
    let columns: std::collections::BTreeSet<String> = tx
        .prepare("SELECT name FROM pragma_table_info(?1)")?
        .query_map([table], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut names = Vec::new();
    let mut values = Vec::new();
    for (name, value) in row {
        if skip.contains(&name.as_str()) || !columns.contains(name) {
            continue;
        }
        names.push(format!("\"{name}\""));
        values.push(match value {
            SidecarValue::Null => Value::Null,
            SidecarValue::Integer(n) => Value::Integer(*n),
            SidecarValue::Real(x) => Value::Real(*x),
            SidecarValue::Text(t) => Value::Text(t.clone()),
            SidecarValue::Blob(h) => Value::Blob(unhex(h)?),
        });
    }
    let placeholders: Vec<String> = (1..=names.len()).map(|i| format!("?{i}")).collect();
    tx.execute(
        &format!(
            "INSERT INTO {table} ({}) VALUES ({})",
            names.join(", "),
            placeholders.join(", ")
        ),
        rusqlite::params_from_iter(values),
    )?;
    Ok(())
}

/// Temp file, `sync_all`, then rename — atomic on both targets. `sync_all` runs before the
/// rename so a power cut cannot leave a renamed-but-empty file.
///
/// # Errors
/// Fails when the document does not serialise, or the directory or temp file cannot be
/// created, written, synced or renamed into place.
pub fn write_atomically(sidecar: &Sidecar, path: &Path) -> Result<(), IndexError> {
    use std::io::Write as _;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);

    let bytes =
        serde_json::to_vec_pretty(sidecar).map_err(|e| IndexError::Sidecar(e.to_string()))?;
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// What a sidecar file is, as §48.8.2's reader finds it. Never an error: each state is an
/// answer a rebuild and the pre-rebuild window both act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarState {
    /// No file exists at the path — `NotFound` and nothing else.
    Absent,
    /// A file exists and cannot be used: any other I/O error, a parse failure or a checksum
    /// mismatch. Nothing is restored from it, and the file is kept.
    Unreadable {
        /// Why, in the reader's words.
        reason: String,
    },
    /// Written by a newer build: an unknown format, a schema above the reader's, or a section
    /// this build does not register. A rebuild restores nothing rather than part of it.
    Newer {
        /// Which of the three, with the value found.
        reason: String,
    },
    /// A document this build reads in full, checksum verified.
    Present(Box<Sidecar>),
}

/// The fields a newer document is recognised by, read before the full parse: a newer writer's
/// records need not parse as this build's.
#[derive(serde::Deserialize)]
struct Header {
    format: u32,
    #[serde(default)]
    schema_version: Option<u32>,
    #[serde(default)]
    payload: HeaderPayload,
}

#[derive(Default, serde::Deserialize)]
struct HeaderPayload {
    #[serde(default)]
    sections: serde_json::Map<String, serde_json::Value>,
}

/// Read the sidecar at `path` into one of its four states, against `supported`, the highest
/// schema version this build migrates to.
///
/// The `newer` checks run before the checksum: a newer writer's payload is not one this build
/// can re-serialise to check, and calling it unreadable would offer a rebuild that drops it.
#[must_use]
pub fn inspect(path: &Path, supported: u32) -> SidecarState {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SidecarState::Absent,
        Err(e) => {
            return SidecarState::Unreadable {
                reason: e.to_string(),
            }
        }
    };
    let header: Header = match serde_json::from_slice(&bytes) {
        Ok(header) => header,
        Err(e) => {
            return SidecarState::Unreadable {
                reason: e.to_string(),
            }
        }
    };
    if header.format != 1 && header.format != SIDECAR_FORMAT {
        return SidecarState::Newer {
            reason: format!(
                "sidecar format {} is not one this build reads",
                header.format
            ),
        };
    }
    if let Some(version) = header.schema_version.filter(|v| *v > supported) {
        return SidecarState::Newer {
            reason: format!("sidecar schema {version} is above this build's {supported}"),
        };
    }
    if let Some(name) = header
        .payload
        .sections
        .keys()
        .find(|name| !SECTIONS.iter().any(|s| s.name == name.as_str()))
    {
        return SidecarState::Newer {
            reason: format!("sidecar section {name} is not registered in this build"),
        };
    }
    let doc: Sidecar = match serde_json::from_slice(&bytes) {
        Ok(doc) => doc,
        Err(e) => {
            return SidecarState::Unreadable {
                reason: e.to_string(),
            }
        }
    };
    match checksum_of(&doc.payload) {
        Ok(expected) if expected == doc.checksum => SidecarState::Present(Box::new(doc)),
        Ok(_) => SidecarState::Unreadable {
            reason: "sidecar checksum does not match its payload".to_owned(),
        },
        Err(e) => SidecarState::Unreadable {
            reason: e.to_string(),
        },
    }
}

/// [`inspect`] for a caller that wants the document or a reason: the document when it is
/// present, otherwise the state as an error.
///
/// # Errors
/// Fails with [`IndexError::Sidecar`] when the sidecar is absent, unreadable or newer.
pub fn read(path: &Path) -> Result<Sidecar, IndexError> {
    match inspect(path, super::migrate::SUPPORTED_SCHEMA_VERSION) {
        SidecarState::Present(doc) => Ok(*doc),
        SidecarState::Absent => Err(IndexError::Sidecar(format!(
            "no sidecar at {}",
            path.display()
        ))),
        SidecarState::Unreadable { reason } | SidecarState::Newer { reason } => {
            Err(IndexError::Sidecar(reason))
        }
    }
}

/// Rows a restore wrote, per kind. An insert skipped because its row already exists counts zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct RestoreCounts {
    /// Scan roots inserted.
    pub roots: u64,
    /// Identities inserted.
    pub identities: u64,
    /// Identity aliases inserted.
    pub aliases: u64,
    /// `app_meta` entries written.
    pub settings: u64,
    /// `view_state` entries written.
    pub view_state: u64,
    /// Collection definitions inserted.
    pub collections: u64,
}

/// Rows affected, as a count. `Connection::execute` returns `usize`.
pub(crate) fn rows(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

/// Bytes back from the hex a blob or path key travels in; fails on an odd length or a non-hex
/// digit.
pub(crate) fn unhex(s: &str) -> Result<Vec<u8>, IndexError> {
    let raw = s.as_bytes();
    if raw.len() % 2 != 0 {
        return Err(IndexError::Sidecar(format!("not hex: {s}")));
    }
    let mut out = Vec::with_capacity(raw.len() / 2);
    for pair in raw.chunks_exact(2) {
        let text = std::str::from_utf8(pair).map_err(|e| IndexError::Sidecar(e.to_string()))?;
        out.push(u8::from_str_radix(text, 16).map_err(|e| IndexError::Sidecar(e.to_string()))?);
    }
    Ok(out)
}

/// Everything that attaches to no project: roots, identities, settings, view state and
/// collection definitions.
///
/// Safe to run twice — every insert is `ON CONFLICT DO NOTHING` against the natural key the DDL
/// already declares. Deleting first would discard rows the user made after the rebuild.
///
/// # Errors
/// Fails when a root's `path_hex` is not hex or SQLite refuses an insert.
pub fn restore_global(conn: &Connection, doc: &Sidecar) -> Result<RestoreCounts, IndexError> {
    let mut c = RestoreCounts::default();

    for root in &doc.payload.roots {
        let bytes = unhex(&root.path_hex)?;
        // path_key is recomputed rather than exported: it is a pure function of the bytes.
        let key = crate::index::path::StoredPath::from_bytes(
            bytes,
            if root.kind == "win" {
                crate::index::path::PathPlatform::Windows
            } else {
                crate::index::path::PathPlatform::Unix
            },
        );
        let (path_bytes, path_key, path_display) = key.as_params();
        c.roots += rows(conn.execute(
            "INSERT INTO scan_root
               (kind, distro, path_bytes, path_key, path_display, enabled, added_by,
                descend_into_repos, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT (kind, distro, path_key) DO NOTHING",
            rusqlite::params![
                root.kind,
                root.distro,
                path_bytes,
                path_key,
                path_display,
                i64::from(root.enabled),
                root.added_by,
                i64::from(root.descend_into_repos),
                root.added_at,
            ],
        )?);
    }

    for identity in &doc.payload.identities {
        c.identities += rows(conn.execute(
            "INSERT INTO identity (email, name, is_user, source, confirmed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (email) DO NOTHING",
            rusqlite::params![
                identity.email,
                identity.name,
                i64::from(identity.is_user),
                identity.source,
                identity.confirmed_at,
            ],
        )?);
        for alias in &identity.aliases {
            c.aliases += rows(conn.execute(
                "INSERT INTO identity_alias (identity_id, email, reason)
                 SELECT id, ?2, ?3 FROM identity WHERE email = ?1
                 ON CONFLICT (email) DO NOTHING",
                rusqlite::params![identity.email, alias.email, alias.reason],
            )?);
        }
    }

    for (k, v) in &doc.payload.settings {
        if never_travels(k) {
            continue;
        }
        c.settings += rows(conn.execute(
            "INSERT INTO app_meta (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            rusqlite::params![k, v],
        )?);
    }
    for (k, v) in &doc.payload.view_state {
        c.view_state += rows(conn.execute(
            "INSERT INTO view_state (k, v) VALUES (?1, ?2)
             ON CONFLICT(k) DO UPDATE SET v = excluded.v",
            rusqlite::params![k, v],
        )?);
    }

    for collection in &doc.payload.collections {
        c.collections += rows(conn.execute(
            "INSERT INTO collection (name, kind, query_text, query_grammar_version, sort_index)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (name) DO NOTHING",
            rusqlite::params![
                collection.name,
                collection.kind,
                collection.query_text,
                collection.query_grammar_version,
                collection.sort_index,
            ],
        )?);
    }
    Ok(c)
}

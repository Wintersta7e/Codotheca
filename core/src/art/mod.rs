//! Card art (§7). The generator is a pure function producing a scene document; the rasterizer
//! walks that document and nothing else. Nothing in this module reads a clock: `now` arrives as
//! a parameter and is only ever stamped into `art_scene.rendered_at`, which is why
//! `scene_hash` cannot become a function of wall-clock time (§7.3a, criterion 62).

pub mod blueprint;
pub mod commands;
pub mod compose;
pub mod derive;
pub mod encode;
pub mod fade;
pub mod generate;
pub mod job;
pub mod oklch;
pub mod raster;
pub mod rename;
pub mod scene;
pub mod seed;
pub mod store;
/// Test doubles for this module. Gated like `core::testing` so no double ships.
#[cfg(feature = "testkit")]
pub mod testsupport;

use std::path::{Path, PathBuf};

use crate::index::{Index, IndexError};
use crate::proto::dispatch::CommandFailure;
use crate::proto::EventSink;
use crate::protocol::{ErrorCode, Rendition};

/// The renderer schema version. It is serialised **into** the scene document (ruling 2), so a
/// bump changes every `scene_hash` and §7.5's lazy re-render sweeps the shelf.
///
/// Bump this whenever any of these changes: the draw order or any constant in `raster.rs`, the
/// plate multipliers, the greebling table, the composition rules in `compose.rs`, or the shape
/// of `Scene` itself.
pub const ART_SCHEMA_VERSION: u32 = 1;

/// §7.3's `"v"`. The *document format*; distinct from `ART_SCHEMA_VERSION`, which is the
/// *renderer*. Phase 3 reads this document as the geometry sidecar and needs to know which
/// shape it is reading, independently of which rasterizer drew it.
pub const SCENE_FORMAT_VERSION: u32 = 1;

/// §7.5: heroes are evicted least-recently-opened beyond this many renditions.
pub const HERO_CACHE_MAX: usize = 200;

/// Ruling 6: art work is scheduled against its own store key.
pub const ART_STORE_KEY_PREFIX: &str = "art:";

#[must_use]
pub fn art_store_key(store_key: &str) -> String {
    format!("{ART_STORE_KEY_PREFIX}{store_key}")
}

/// Everything an art command or job needs. `now` is unix **seconds**, supplied by the caller so
/// nothing under this module reads the clock itself.
pub struct ArtCtx<'a> {
    pub index: &'a Index,
    pub events: &'a dyn EventSink,
    pub now: i64,
}

impl std::fmt::Debug for ArtCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtCtx")
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub enum ArtError {
    Index(IndexError),
    /// A stale or tombstoned project id (§1.6). Kept as its own variant so
    /// `ErrorCode::ProjectMerged` reaches the wire: collapsing it into `Internal` would
    /// tell a rail holding a pre-merge id to retry rather than to refresh.
    Identity(crate::identity::IdentityError),
    Sqlite(rusqlite::Error),
    Io(String),
    Encode(String),
    /// A hash that is not 64 lowercase hex. Never built from user input without this check.
    BadHash(String),
    /// No `art_scene` row for the project.
    NoScene(i64),
}

impl std::fmt::Display for ArtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Index(e) => write!(f, "index: {e:?}"),
            Self::Identity(e) => write!(f, "identity: {e:?}"),
            Self::Sqlite(e) => write!(f, "sqlite: {e}"),
            Self::Io(d) => write!(f, "io: {d}"),
            Self::Encode(d) => write!(f, "encode: {d}"),
            Self::BadHash(h) => write!(f, "not a scene hash: {h}"),
            Self::NoScene(id) => write!(f, "no scene for project {id}"),
        }
    }
}

impl std::error::Error for ArtError {}

impl From<rusqlite::Error> for ArtError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}

impl From<IndexError> for ArtError {
    fn from(e: IndexError) -> Self {
        Self::Index(e)
    }
}

impl From<crate::identity::IdentityError> for ArtError {
    fn from(e: crate::identity::IdentityError) -> Self {
        Self::Identity(e)
    }
}

impl ArtError {
    /// The core's `message` is diagnostic and never shown raw; this is the closed code the
    /// shell narrows on.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::BadHash(_) | Self::NoScene(_) => ErrorCode::Protocol,
            Self::Identity(e) => e.code(),
            _ => ErrorCode::Internal,
        }
    }
}

/// §7.6's path segment, one per rendition. R47: the blueprint pass needs **two** names, because
/// the address is `codotheca://art/<hash>/<rendition>` — with one, a cached raster of the card
/// pass would be served for the hero pass at exactly the moment the project changes state.
#[must_use]
pub fn rendition_slug(r: Rendition) -> &'static str {
    match r {
        Rendition::Card => "card",
        Rendition::Hero => "hero",
        Rendition::CardBlueprint => "card-blueprint",
        Rendition::HeroBlueprint => "hero-blueprint",
    }
}

#[must_use]
pub fn rendition_from_slug(s: &str) -> Option<Rendition> {
    match s {
        "card" => Some(Rendition::Card),
        "hero" => Some(Rendition::Hero),
        "card-blueprint" => Some(Rendition::CardBlueprint),
        "hero-blueprint" => Some(Rendition::HeroBlueprint),
        _ => None,
    }
}

/// 64 lowercase hex and nothing else. This is the only guard between a wire string and a
/// filesystem path, so it is total: no separators, no case folding, no length slack.
#[must_use]
pub fn is_scene_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[must_use]
pub fn art_root(data_dir: &Path) -> PathBuf {
    data_dir.join("art")
}

/// `<data_dir>/art/<aa>/<hash>.<rendition>.webp` — §7.2's two-level fan-out with §7.6's
/// rendition segment. `None` for anything that is not a scene hash.
#[must_use]
pub fn rendition_path(data_dir: &Path, hash: &str, r: Rendition) -> Option<PathBuf> {
    if !is_scene_hash(hash) {
        return None;
    }
    let (fan, _) = hash.split_at(2);
    Some(
        art_root(data_dir)
            .join(fan)
            .join(format!("{hash}.{}.webp", rendition_slug(r))),
    )
}

/// §7.2's privileged scheme. Two segments after the host, always.
#[must_use]
pub fn art_url(hash: &str, r: Rendition) -> Option<String> {
    if !is_scene_hash(hash) {
        return None;
    }
    Some(format!("codotheca://art/{hash}/{}", rendition_slug(r)))
}

// R15: `parse_args` is **not** declared here. Plan 03 declares the one helper beside
// `CommandFailure` in `core/src/proto/dispatch.rs`; every caller under `core/src/art`
// imports it directly as `use crate::proto::dispatch::parse_args;`. It is deliberately not
// re-exported from this module, so there is one path to it and not two. Plan 03's
// `core::lifecycle::parse_args` parses *argv* and is a different, unrelated function.

/// The commands this module owns, in the order the dispatcher matches them. Exposed so the
/// dispatch table can be asserted without constructing an `Index`.
#[must_use]
pub fn dispatch_art_command_names() -> [&'static str; 2] {
    ["art.url", "art.rerender"]
}

/// `None` means "not mine". A later plan chains its own dispatcher after this one.
#[must_use]
pub fn dispatch_art_command(
    ctx: &ArtCtx<'_>,
    command: &str,
    args: serde_json::Value,
) -> Option<Result<serde_json::Value, CommandFailure>> {
    match command {
        "art.url" => Some(commands::handle_url(ctx, args)),
        "art.rerender" => Some(commands::handle_rerender(ctx, args)),
        _ => None,
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
    use crate::protocol::{ErrorCode, Rendition};
    use std::path::Path;

    const H: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_scene_hash_is_sixty_four_lowercase_hex_and_nothing_else() {
        assert!(is_scene_hash(H));
        // Anything that could walk out of the art tree, or name a file that is not ours.
        assert!(!is_scene_hash(""));
        assert!(!is_scene_hash(".."));
        assert!(!is_scene_hash("../../etc/passwd"));
        assert!(!is_scene_hash(
            "0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(!is_scene_hash("0123456789abcdef"));
        assert!(!is_scene_hash(&format!("{H}0")));
        assert!(!is_scene_hash(&format!("{}/x", &H[..62])));
    }

    #[test]
    fn the_address_carries_the_rendition_because_one_filename_cannot_hold_both() {
        // §7.2/§7.6: `<appdata>/art/<aa>/<scene_hash>.<rendition>.webp`, two-level fan-out.
        let root = Path::new("/data");
        let card = rendition_path(root, H, Rendition::Card).expect("card path");
        let hero = rendition_path(root, H, Rendition::Hero).expect("hero path");
        assert_ne!(card, hero);
        assert!(card.ends_with(format!("{H}.card.webp")));
        assert!(hero.ends_with(format!("{H}.hero.webp")));
        assert_eq!(
            card.parent().and_then(std::path::Path::file_name),
            Some("01".as_ref())
        );
        assert_eq!(art_root(root), Path::new("/data").join("art"));
    }

    #[test]
    fn a_path_is_never_built_from_a_hash_that_is_not_one() {
        assert!(rendition_path(Path::new("/data"), "..", Rendition::Card).is_none());
        assert!(art_url("..", Rendition::Card).is_none());
    }

    #[test]
    fn the_url_has_two_segments_after_the_host() {
        // §7.6, criterion 62: a fetch of the hash alone must fail, so the address is built
        // with the rendition or it is not built at all.
        assert_eq!(
            art_url(H, Rendition::Hero),
            Some(format!("codotheca://art/{H}/hero"))
        );
        assert_eq!(rendition_slug(Rendition::Card), "card");
        assert_eq!(rendition_from_slug("hero"), Some(Rendition::Hero));
        assert_eq!(rendition_from_slug("thumbnail"), None);
    }

    /// R47: one variant cannot address two render passes. The hazard §23.5 names is a cached
    /// raster of one pass being served for the other at exactly the moment the project changes
    /// state, and a distinct one-segment slug per pass is what closes it.
    #[test]
    fn the_two_blueprint_passes_have_their_own_addresses_and_their_own_files() {
        assert_eq!(
            art_url(H, Rendition::CardBlueprint),
            Some(format!("codotheca://art/{H}/card-blueprint"))
        );
        assert_ne!(
            art_url(H, Rendition::CardBlueprint),
            art_url(H, Rendition::Card)
        );
        assert_ne!(
            art_url(H, Rendition::CardBlueprint),
            art_url(H, Rendition::HeroBlueprint)
        );
        assert_ne!(
            art_url(H, Rendition::HeroBlueprint),
            art_url(H, Rendition::Hero)
        );

        // The slugs round-trip, so a swept file's name resolves back to the rendition that wrote
        // it — which is what `sweep_unreferenced` reads.
        for r in [
            Rendition::Card,
            Rendition::Hero,
            Rendition::CardBlueprint,
            Rendition::HeroBlueprint,
        ] {
            assert_eq!(rendition_from_slug(rendition_slug(r)), Some(r));
            // One path segment each: a slug carrying a separator would break §7.6's two-segment
            // address and `renditionFilePath`'s containment check at once.
            assert!(!rendition_slug(r).contains('/'));
            assert!(!rendition_slug(r).contains('.'));
        }

        // Four distinct files under one hash.
        let paths: std::collections::BTreeSet<_> = [
            Rendition::Card,
            Rendition::Hero,
            Rendition::CardBlueprint,
            Rendition::HeroBlueprint,
        ]
        .into_iter()
        .filter_map(|r| rendition_path(Path::new("/data"), H, r))
        .collect();
        assert_eq!(paths.len(), 4, "two passes must not share a file");
    }

    #[test]
    fn art_jobs_take_a_slot_keyed_apart_from_the_repositorys_own() {
        // Ruling 6: J5 runs no git, so it must not spend a repository store's job slot.
        assert_eq!(art_store_key("vol-a"), "art:vol-a");
        assert_ne!(art_store_key("vol-a"), "vol-a");
    }

    #[test]
    fn dispatch_declines_a_command_this_module_does_not_own() {
        assert!(dispatch_art_command_names()
            .iter()
            .all(|n| n.starts_with("art.")));
        assert_eq!(dispatch_art_command_names(), ["art.url", "art.rerender"]);
    }

    #[test]
    fn a_stale_project_id_keeps_its_own_wire_code_instead_of_becoming_internal() {
        // §1.6: a rail holding a pre-merge id must be told to refresh, not to retry.
        let err = ArtError::Identity(crate::identity::IdentityError::ProjectMerged {
            requested: 3,
            into: 4,
        });
        assert_eq!(err.code(), ErrorCode::ProjectMerged);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupReport {
    pub staled: usize,
    pub swept: store::SweepReport,
}

/// Run once per process, before any command is served: §7.5's schema-bump stale pass and the
/// sweep of files no `art_scene` row references. Nothing is re-rendered here — that is lazy and
/// shelf-visible first.
///
/// Wire it into the core's start-up path immediately after the index is opened and before the
/// transport loop begins.
pub fn startup(ctx: &ArtCtx<'_>) -> Result<StartupReport, ArtError> {
    let staled = store::mark_stale_on_schema_bump(ctx.index.conn(), ART_SCHEMA_VERSION)?;
    let swept = store::sweep_unreferenced(ctx.index.conn(), ctx.index.data_dir())?;
    Ok(StartupReport { staled, swept })
}

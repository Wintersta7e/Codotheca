//! `projects.readmeAssets` — classification, the caps, and the local branch.
//!
//! One command answers both branches because the renderer parsed one list of `img[src]` values
//! and does not know which is which. **The arrays are a hint; the core's classification is the
//! decision**, so a value the renderer put in `local` that parses as a URL is handled as remote.
//!
//! **Answered without the index guard** (R75, `Route::ReadmeNet`). The remote branch makes up to
//! [`ASSET_COUNT_CAP`] calls of [`crate::readme::fetch::REMOTE_TIMEOUT_SECS`] each, and holding
//! the process's one SQLite mutex across them would stop every other command for the duration —
//! the wedge `f182452` fixed, arriving from the other side. What this module holds instead is
//! [`AssetDeps`]: the two values that had to be read from the index, read once, before the first
//! socket. **It cannot touch the index because it is not given one** (R100).

use std::path::{Component, Path, PathBuf};

use crate::http::HttpTransport;
use crate::protocol::{ReadmeAsset, ReadmeAssetState};
use crate::readme::fetch::{fetch_remote_asset, HostResolver};

/// §25.5's per-asset cap.
pub const ASSET_BYTE_CAP: usize = 512 * 1024;
/// §25.5's cap on references accepted per call. The 25th and beyond are dropped from the reply
/// entirely, so the panel keeps its placeholder rather than being told something false about it.
pub const ASSET_COUNT_CAP: usize = 24;
/// §25.5's cap on accumulated **source** bytes per reply, applied before base64.
///
/// The arithmetic is what keeps the reply inside `MAX_FRAME_BYTES` (8 MiB,
/// `app/src/main/core/frame.ts:8`): 4 MB of source encodes to ~5.33 MB of base64. Enforcing it
/// after encoding would be a cap on the wrong number.
pub const REPLY_BYTE_CAP: usize = 4 * 1024 * 1024;

/// The five media types §25.5 admits.
///
/// `image/svg+xml` is here because badges are SVG and an SVG loaded through `<img>` is a replaced
/// element: script inside it never executes. Inline `<svg>` is a different thing and the
/// renderer's allowlist forbids it.
pub const ALLOWED_MEDIA_TYPES: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/svg+xml",
];

/// What the core decided a reference is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetRef {
    /// A path to resolve under the location's working directory.
    Local(String),
    /// An `https:` URL.
    Remote(Box<reqwest::Url>),
    /// Nothing this build will read, with the state to render.
    Rejected(ReadmeAssetState),
}

/// Classify one reference.
///
/// A reference is **remote** when it parses as an absolute URL with a scheme; anything else is
/// **local**. A remote reference whose scheme is not `https:` is `unreachable` and is never
/// fetched — `http:` included, and `data:` with it: bytes this build did not produce do not
/// become an image because the document says they are one.
#[must_use]
pub fn classify_ref(raw: &str) -> AssetRef {
    match reqwest::Url::parse(raw) {
        Ok(url) if url.scheme() == "https" => AssetRef::Remote(Box::new(url)),
        Ok(_) => AssetRef::Rejected(ReadmeAssetState::Unreachable),
        // **Decoded here, before anything looks at its components.** `markdown-it` percent-encodes
        // a link destination, so `docs/架构.png` arrives as `docs/%E6%9E%B6%E6%9E%84.png` and a
        // space arrives as `%20`; joined literally, neither file is ever found and an ordinary
        // repository's own diagram is reported `not_an_image` — a false claim about a file that
        // exists. The reply still echoes the **original** reference, because that is what the
        // renderer matches its placeholder on.
        Err(_) => AssetRef::Local(percent_decoded(raw)),
    }
}

/// One pass of `%XX` decoding, or the input unchanged.
///
/// **Order matters and is the whole safety argument**: this runs in `classify_ref`, *before*
/// [`resolve_local_asset`]'s lexical clause, so `..%2Fsecret.png` becomes `../secret.png` and is
/// then refused as the `..` it is. Decoding *after* that clause would walk it straight through.
///
/// One pass, never recursive: `%252e` decodes to `%2e` and stops, so a doubly-encoded traversal
/// cannot be assembled by decoding twice. A sequence that is not valid UTF-8 once decoded is left
/// alone — a filename this build cannot name in a `String` is one it does not open.
#[must_use]
pub fn percent_decoded(raw: &str) -> String {
    if !raw.contains('%') {
        return raw.to_owned();
    }
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes.get(index).copied().unwrap_or(b'\0');
        let decoded = if byte == b'%' {
            let hex = raw.get(index + 1..index + 3);
            hex.and_then(|hex| u8::from_str_radix(hex, 16).ok())
        } else {
            None
        };
        if let Some(value) = decoded {
            out.push(value);
            index += 3;
        } else {
            out.push(byte);
            index += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| raw.to_owned())
}

/// The media type of `bytes`, sniffed from the bytes themselves.
///
/// Never from a file extension and never from a `Content-Type` header: both are claims by
/// somebody else about what the payload is, and this decides whether the payload reaches a
/// document.
///
/// **The SVG sniff is a heuristic and is stated as one.** SVG has no magic number, so a leading
/// `<svg` — or an XML declaration or comment followed by `<svg` inside the first kilobyte — is
/// the whole test. Anything it does not recognise is `not_an_image`, which is the safe direction.
#[must_use]
pub fn sniff_media_type(bytes: &[u8]) -> Option<&'static str> {
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.starts_with(PNG) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Some("image/webp");
    }
    let head = bytes.get(..bytes.len().min(1024)).unwrap_or(bytes);
    let text = String::from_utf8_lossy(head);
    let trimmed = text.trim_start();
    if trimmed.starts_with("<svg") || (trimmed.starts_with('<') && trimmed.contains("<svg")) {
        return Some("image/svg+xml");
    }
    None
}

/// Resolve a local reference under `root`, or say why it is refused.
///
/// Three clauses, in this order, and the order is the guarantee:
///
/// 1. **Lexically**, before touching the disk: no absolute path, no drive or UNC prefix, no `..`
///    component. A hostile reference never becomes a `stat` on an attacker-chosen path.
/// 2. `canonicalize` both sides and require containment. **This is the clause the TypeScript
///    predicate §25.5 cites does not have**: `app/src/main/art/artProtocol.ts:91-106` is
///    `path.resolve(file).startsWith(path.resolve(root) + path.sep)`, which is purely lexical and
///    cannot see a symlink that leaves the root. Clause 1 refuses the traversal; clause 2 refuses
///    the escape.
/// 3. The canonical target is a **regular file** — not a directory, not a FIFO, not a device.
///
/// # Errors
/// `not_an_image` for every refusal. A README that points at `../../etc/passwd` is a README, and
/// refusing to render one image is the correct outcome rather than failing the whole panel.
pub fn resolve_local_asset(root: &Path, reference: &str) -> Result<PathBuf, ReadmeAssetState> {
    if reference.is_empty() {
        return Err(ReadmeAssetState::NotAnImage);
    }
    let candidate = Path::new(reference);
    if candidate.is_absolute() {
        return Err(ReadmeAssetState::NotAnImage);
    }
    for component in candidate.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            // `Prefix` is a drive letter or a UNC share, `RootDir` a leading separator, and
            // `ParentDir` is the traversal itself. A Windows path also reaches here on a Unix
            // host, where `C:\x` is one `Normal` component — which clause 2 then refuses,
            // because no such file exists under the root.
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => {
                return Err(ReadmeAssetState::NotAnImage);
            }
        }
    }

    let canonical_root = root
        .canonicalize()
        .map_err(|_| ReadmeAssetState::NotAnImage)?;
    let target = canonical_root
        .join(candidate)
        .canonicalize()
        .map_err(|_| ReadmeAssetState::NotAnImage)?;
    if !target.starts_with(&canonical_root) {
        return Err(ReadmeAssetState::NotAnImage);
    }
    let meta = std::fs::metadata(&target).map_err(|_| ReadmeAssetState::NotAnImage)?;
    if !meta.is_file() {
        return Err(ReadmeAssetState::NotAnImage);
    }
    Ok(target)
}

/// Everything the asset read needs, and nothing else — no index, no clock, no event sink.
///
/// `consent` is `project.readme_remote_at`, read under the guard before this runs. A value rather
/// than a connection, because this code runs **off** the index lock and must not be able to take
/// it: the borrow checker, not a comment, is what keeps that true.
pub struct AssetDeps<'a> {
    pub work_dir: PathBuf,
    pub consent: Option<i64>,
    pub http: &'a dyn HttpTransport,
    pub resolve: HostResolver,
    pub now: i64,
}

impl std::fmt::Debug for AssetDeps<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetDeps")
            .field("consent", &self.consent)
            .field("now", &self.now)
            .finish_non_exhaustive()
    }
}

/// Read the bytes behind one README's image references.
///
/// The caps apply in §25.5's order: the count cap drops references from the reply entirely, the
/// per-asset cap and the media sniff decide one row each, and the accumulated-source cap makes
/// the first reference to cross it — and every one after — `too_large`.
///
/// References are **de-duplicated on first appearance**: one entry per distinct reference, and
/// therefore at most one request per distinct URL. The renderer matches rows back to nodes by
/// `ref`, so a document naming one badge three times renders three images from one read.
///
/// **The two lists are interleaved, and that is a correctness fix rather than a nicety.** Taking
/// locals first meant a README with 24 or more local images and one badge produced **no row at
/// all** for the badge — so nothing was `blocked`, the panel's consent block is drawn only when
/// something is, and the one control that would let the user load remote images was never
/// rendered. The cap is deliberate; removing the user's only way to act was not.
#[must_use]
pub fn read_readme_assets(
    deps: &AssetDeps<'_>,
    local: &[String],
    remote: &[String],
) -> Vec<ReadmeAsset> {
    let mut seen: Vec<&str> = Vec::new();
    let mut answers: Vec<ReadmeAsset> = Vec::new();
    let mut source_bytes: usize = 0;

    for reference in interleaved(local, remote) {
        if seen.len() >= ASSET_COUNT_CAP {
            break;
        }
        if seen.contains(&reference.as_str()) {
            continue;
        }
        seen.push(reference.as_str());

        let outcome = match classify_ref(reference) {
            AssetRef::Rejected(state) => Err(state),
            AssetRef::Local(path) => read_local(&deps.work_dir, &path),
            AssetRef::Remote(url) => {
                if deps.consent.is_none() {
                    // The consent state, and nothing was observed — so no clock and no socket.
                    Err(ReadmeAssetState::Blocked)
                } else {
                    fetch_remote_asset(deps.http, &url, deps.resolve)
                }
            }
        };

        answers.push(match outcome {
            Ok((bytes, media)) => {
                // The reply cap is on **source** bytes, checked before this asset is admitted:
                // once the budget is spent every later asset is `too_large` too, which is the
                // honest answer — it is the reply that has no room, not this file that is small.
                let projected = source_bytes.saturating_add(bytes.len());
                if projected > REPLY_BYTE_CAP {
                    state_only(reference, ReadmeAssetState::TooLarge)
                } else {
                    source_bytes = projected;
                    ReadmeAsset {
                        r#ref: reference.clone(),
                        state: ReadmeAssetState::Ok,
                        data_uri: Some(data_uri(media, &bytes)),
                        fetched_at: Some(deps.now),
                    }
                }
            }
            Err(state) => state_only(reference, state),
        });
    }

    answers
}

/// The two lists, one from each in turn, so neither can starve the other out of the count cap.
///
/// Order within the reply does not matter — the renderer matches by `ref` — so the only thing this
/// decides is **which** references get a row when there are more than [`ASSET_COUNT_CAP`] of them.
fn interleaved<'a>(local: &'a [String], remote: &'a [String]) -> Vec<&'a String> {
    let mut out: Vec<&String> = Vec::with_capacity(local.len() + remote.len());
    let mut index = 0;
    while index < local.len() || index < remote.len() {
        if let Some(reference) = local.get(index) {
            out.push(reference);
        }
        if let Some(reference) = remote.get(index) {
            out.push(reference);
        }
        index += 1;
    }
    out
}

/// A row carrying a state and nothing else. `dataUri` absent because there are no bytes, and
/// `fetchedAt` absent because nothing was observed.
fn state_only(reference: &str, state: ReadmeAssetState) -> ReadmeAsset {
    ReadmeAsset {
        r#ref: reference.to_owned(),
        state,
        data_uri: None,
        fetched_at: None,
    }
}

fn read_local(root: &Path, reference: &str) -> Result<(Vec<u8>, &'static str), ReadmeAssetState> {
    use std::io::Read as _;
    let path = resolve_local_asset(root, reference)?;
    let file = std::fs::File::open(&path).map_err(|_| ReadmeAssetState::NotAnImage)?;
    let mut bytes = Vec::new();
    // `cap + 1`, so a file over the cap costs one byte over it rather than the whole read, and
    // is still distinguishable from one that ends exactly at it.
    file.take(ASSET_BYTE_CAP as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadmeAssetState::NotAnImage)?;
    if bytes.len() > ASSET_BYTE_CAP {
        return Err(ReadmeAssetState::TooLarge);
    }
    let media = sniff_media_type(&bytes).ok_or(ReadmeAssetState::NotAnImage)?;
    Ok((bytes, media))
}

/// `data:<media>;base64,<payload>`.
fn data_uri(media: &str, bytes: &[u8]) -> String {
    use base64::Engine as _;
    let payload = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{media};base64,{payload}")
}

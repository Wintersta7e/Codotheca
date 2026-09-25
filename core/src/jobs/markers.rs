//! §29.3's marker pass — three markers, raw bytes, no language parsed.
//!
//! **J7 does not parse.** It does not know which language it is reading, so a marker inside a
//! string literal is a finding; a lexer per language is a cost with no measured benefit, and the
//! settled row *gaming is accepted, not engineered against* covers the converse.

use std::sync::OnceLock;

use aho_corasick::AhoCorasick;

use crate::debt::identity::normalise_salient;

/// `concept.md`'s three and only those, matched case-sensitively and upper case.
pub const MARKERS: [&str; 3] = ["TODO", "FIXME", "HACK"];

/// Everything that changes what a blob read produces: the three markers, the boundary rule, J7's
/// extraction, §28.1's normalisation and cap **as J7 applies them**, [`J7_BLOB_BYTE_CAP`] and
/// [`J7_BINARY_SNIFF_BYTES`].
///
/// A change to any of them makes a stored row a **miss, not a hit**, which is the whole reason
/// the cache carries a version. `art_scene.schema_version` is the precedent.
pub const J7_SCANNER_VERSION: i64 = 1;

/// §29.2 rule 4. A blob over this is recorded as `too_large`, never silently dropped — the size
/// comes from `cat-file --batch`'s own header, so the verdict carries its evidence.
pub const J7_BLOB_BYTE_CAP: u64 = 512 * 1024;

/// §29.2 rule 5. A NUL byte inside this prefix makes the blob `binary`.
pub const J7_BINARY_SNIFF_BYTES: usize = 8 * 1024;

/// §29.6's chunk ceiling in blobs. **§29.6 is the one owner of the numeral.**
///
/// Chosen, then **measured and kept**: Probe D, a 20-repository sample, 2026-09-18, mean filtered
/// blob **7,800 bytes**. 512 of them is ~3.8 MB — under [`J7_CHUNK_BYTES`] — so **this ceiling
/// binds first on an ordinary tree** and the byte one binds only on a blob-heavy one, which is
/// what it was written for. At the measured 2.6 MB/s a full chunk is ~1.5 s.
pub const J7_CHUNK_BLOBS: usize = 512;

/// §29.6's chunk ceiling in blob bytes, whichever comes first. Measured and kept on the same
/// terms as [`J7_CHUNK_BLOBS`]: it caps a chunk at ~3.1 s on the tree Probe D measured.
pub const J7_CHUNK_BYTES: u64 = 8 * 1024 * 1024;

/// One of [`MARKERS`], named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Marker {
    Todo,
    Fixme,
    Hack,
}

impl Marker {
    /// Every marker, so a test can walk the vocabulary without restating it.
    pub const ALL: [Self; 3] = [Self::Todo, Self::Fixme, Self::Hack];

    /// The stored form. `blob_finding.marker`'s CHECK mirrors these character for character.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Self::Todo => "TODO",
            Self::Fixme => "FIXME",
            Self::Hack => "HACK",
        }
    }

    /// `slug`'s inverse. `None` for a marker a newer build wrote.
    #[must_use]
    pub fn from_slug(s: &str) -> Option<Self> {
        match s {
            "TODO" => Some(Self::Todo),
            "FIXME" => Some(Self::Fixme),
            "HACK" => Some(Self::Hack),
            _ => None,
        }
    }
}

/// One marker occurrence inside one blob.
///
/// **The name says `Blob` because §28 declares a project-scope `Occurrence`** and §29.3 rules the
/// two ordinals must never be conflated: a blob's ordinal is stable in every project that holds
/// that blob, a project's ordinal is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobOccurrence {
    /// 0-based, ascending on `(line, column)` **within this blob**.
    pub ordinal_in_blob: u32,
    /// Which of the three.
    pub marker: Marker,
    /// 1-based.
    pub line: u32,
    /// 1-based, **in bytes** — J7 does not parse and does not know the encoding.
    pub column: u32,
    /// The lowercase hex SHA-256 of `salient_text_capped`.
    pub salient_sha256: String,
    /// §28.1's normalised, capped text.
    pub salient_text_capped: String,
}

/// The automaton, built once. Rebuilding it per blob would dominate the pass it exists to make
/// fast.
fn automaton() -> &'static AhoCorasick {
    static BUILT: OnceLock<AhoCorasick> = OnceLock::new();
    BUILT.get_or_init(|| {
        AhoCorasick::new(MARKERS).unwrap_or_else(|e| unreachable!("three literal patterns: {e}"))
    })
}

/// A match counts when the byte before it is not `[A-Za-z0-9_]`, or the blob begins there.
///
/// **The byte after is unconstrained**: a trailing boundary would reject `TODOs`, which is a real
/// marker, while the leading one is what rejects `NOTODO`.
fn boundary_before(bytes: &[u8], start: usize) -> bool {
    match start.checked_sub(1).and_then(|i| bytes.get(i)) {
        None => true,
        Some(b) => !(b.is_ascii_alphanumeric() || *b == b'_'),
    }
}

/// Every marker occurrence in `bytes`, in `(line, column)` order.
#[must_use]
pub fn scan_blob(bytes: &[u8]) -> Vec<BlobOccurrence> {
    let mut out: Vec<BlobOccurrence> = Vec::new();
    // The offset of the byte after the last newline seen, and how many lines precede it. Both
    // advance monotonically with the match offsets, so the whole pass stays linear.
    let mut line_start = 0_usize;
    let mut line_number = 1_u32;
    let mut scanned = 0_usize;

    for hit in automaton().find_iter(bytes) {
        let start = hit.start();
        if !boundary_before(bytes, start) {
            continue;
        }
        let Some(marker) = MARKERS
            .get(hit.pattern().as_usize())
            .and_then(|literal| Marker::from_slug(literal))
        else {
            continue;
        };
        for (offset, byte) in bytes.iter().enumerate().skip(scanned).take(start - scanned) {
            if *byte == b'\n' {
                line_number = line_number.saturating_add(1);
                line_start = offset + 1;
            }
        }
        scanned = start;

        // The salient is the bytes from the marker's first byte to the end of its line. That is
        // the whole of the producer's rule; §28.1 owns what happens to them next.
        let end = bytes
            .get(start..)
            .and_then(|rest| rest.iter().position(|b| *b == b'\n'))
            .map_or(bytes.len(), |at| start + at);
        let salient_text_capped = normalise_salient(bytes.get(start..end).unwrap_or_default());
        out.push(BlobOccurrence {
            ordinal_in_blob: u32::try_from(out.len()).unwrap_or(u32::MAX),
            marker,
            line: line_number,
            column: u32::try_from(start - line_start + 1).unwrap_or(u32::MAX),
            salient_sha256: sha256_hex(salient_text_capped.as_bytes()),
            salient_text_capped,
        });
    }
    out
}

/// Lowercase hex SHA-256, of the **capped** text — §28.1's cap is applied before the hash and not
/// after, so a caller that hashed the raw bytes would produce a different identity.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    use std::fmt::Write as _;
    let mut out = String::with_capacity(64);
    for byte in sha2::Sha256::digest(bytes) {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

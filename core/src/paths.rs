//! The three forms §1.3 stores for every path, as free functions, and the one comparison the
//! scanner needs.
//!
//! **The rules live in [`crate::index::path`]; this module is the free-function surface over
//! them.** `StoredPath` already owns the byte encoding and the canonical-key rule, and a second
//! implementation of either is the defect this project has recorded most often — one value
//! stated twice, drifting. So `path_bytes` and `path_key` delegate, and a test asserts they
//! agree with `StoredPath` byte for byte.
//!
//! `path_bytes` is the operational path and is lossless on Unix. `path_key` is the comparison
//! form — separators normalised, repeats collapsed, case-folded when the **caller-supplied**
//! `PathPlatform` is `Windows` (R2) — and is never used to open anything. `path_display` is lossy
//! and is for the UI alone (§1.10: it is write-once and never read back).

use std::path::{Path, PathBuf};

use crate::index::path::{os_bytes, PathPlatform, StoredPath};

/// Byte encoding of a path, in the one encoding `location.path_bytes` holds.
///
/// Lossless on Unix, where a filename is arbitrary bytes. On a Windows host an `OsStr` is
/// UTF-16 and there is no byte view to borrow, so it is the UTF-8 form — which is what
/// `StoredPath` already writes, and the two must not disagree.
#[must_use]
pub fn path_bytes(path: &Path) -> Vec<u8> {
    os_bytes(path.as_os_str())
}

/// Inverse of [`path_bytes`].
#[must_use]
pub fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;
        PathBuf::from(OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// Canonical comparison form for a path the host itself can open.
///
/// **R2: the platform is an argument, never `#[cfg]`.** The host does not decide how a path is
/// compared; the path's own filesystem does. A Windows host that folded a `\\wsl$\…` Linux path
/// would merge two directories, and neither behaviour could be tested off its own host. Callers
/// looking at the host's own filesystem pass `native_platform()`; callers that know the path's
/// origin (a `scan_root.kind` of `win` vs `linux`/`wsl`) pass it explicitly. Bytes that arrived
/// from *another* machine — plan 18's in-distro worker — go through
/// `StoredPath::from_bytes(bytes, platform)` instead: this function only ever sees a `Path`.
#[must_use]
pub fn path_key(path: &Path, platform: PathPlatform) -> Vec<u8> {
    StoredPath::from_os(path.as_os_str(), platform)
        .key()
        .to_vec()
}

/// True when a displayed path begins `X:`, which is what makes it a Windows path.
///
/// Read off the path itself and never off the host (**R2**): the core indexes Windows paths from
/// a Linux worker and Linux paths from a Windows host, so `cfg!(windows)` would answer about the
/// wrong machine.
fn has_drive_letter(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.get(1) == Some(&b':') && bytes.first().is_some_and(u8::is_ascii_alphabetic)
}

/// Lossy, for the UI only. Never used to open, launch or compare (§1.3).
///
/// **One separator per path, chosen by the path.** A scan root stored as `C:/P` and joined with a
/// child rendered as `C:/P\0` on screen — two separators in one string, which reads as a corrupt
/// value rather than a location, and was reported as a defect against a path that was perfectly
/// valid. `Path::join` appends the *host's* separator regardless of how the root was spelled, so
/// the mix is produced at display time and has to be resolved there.
///
/// Only a drive-lettered path is rewritten. A UNC share and §4bis.4's `\\wsl.localhost\…` form
/// already carry backslashes, and a POSIX path must keep its forward slashes — rewriting either
/// by the host's convention is how a WSL path gets shown as a Windows one.
#[must_use]
pub fn path_display(path: &Path) -> String {
    let raw = path.to_string_lossy().into_owned();
    if has_drive_letter(&raw) {
        raw.replace('/', "\\")
    } else {
        raw
    }
}

/// True when `child_key` is `parent_key` or lies beneath it on a component boundary.
/// Both arguments must already be [`path_key`] output.
#[must_use]
pub fn is_under(child_key: &[u8], parent_key: &[u8]) -> bool {
    if child_key == parent_key {
        return true;
    }
    if parent_key == b"/" {
        return child_key.first() == Some(&b'/');
    }
    child_key.len() > parent_key.len()
        && child_key.starts_with(parent_key)
        && child_key.get(parent_key.len()) == Some(&b'/')
}

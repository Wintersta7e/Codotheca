//! §4bis.4 — path translation, in both directions.
//!
//! Two independent pairs live here. Bridge form (`\\wsl.localhost\<distro>\…`) is a *display and
//! launch* form only: §4.5 forbids scanning or running git through it. `DrvFS` form (`/mnt/c/…`)
//! is a Windows volume surfaced inside the distro, and which mount point carries it is a fact
//! read from the mount table, never a name matched with a pattern.

pub(crate) const WSL_DOLLAR: &str = r"\\wsl$\";
pub(crate) const WSL_LOCALHOST: &str = r"\\wsl.localhost\";

/// A path on the WSL bridge, split into the distro that owns it and the path inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePath {
    /// The distro the bridge path's first component names, case kept.
    pub distro: String,
    /// The absolute path inside the distro, `/`-separated.
    pub linux_path: String,
}

/// `Some` when `raw` is a bridge path. Both the legacy `\\wsl$` host and the current
/// `\\wsl.localhost` host are accepted; only the latter is ever produced.
///
/// A Linux filename may legally contain a backslash, and such a name cannot be represented
/// unambiguously on the bridge. Every backslash is read as a separator here, which is the same
/// reading Explorer gives the path.
#[must_use]
pub fn parse_bridge_path(raw: &str) -> Option<BridgePath> {
    let norm = raw.replace('/', "\\");
    let lower = norm.to_ascii_lowercase();
    // Both prefixes are ASCII, so a byte offset taken from the lowercased copy is valid on the
    // original.
    let rest_at = if lower.starts_with(WSL_DOLLAR) {
        WSL_DOLLAR.len()
    } else if lower.starts_with(WSL_LOCALHOST) {
        WSL_LOCALHOST.len()
    } else {
        return None;
    };
    let mut parts = norm.get(rest_at..)?.split('\\').filter(|s| !s.is_empty());
    let distro = parts.next()?.to_owned();
    let tail: Vec<&str> = parts.collect();
    let linux_path = if tail.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", tail.join("/"))
    };
    Some(BridgePath { distro, linux_path })
}

/// The bridge form of a path inside a distro. **Display and launch only** (§4bis.4): never pass
/// this to the walk, to git, or to `location.path_bytes`.
#[must_use]
pub fn bridge_path(distro: &str, linux_path: &str) -> String {
    let tail = linux_path.trim_start_matches('/').replace('/', "\\");
    if tail.is_empty() {
        format!(r"\\wsl.localhost\{distro}")
    } else {
        format!(r"\\wsl.localhost\{distro}\{tail}")
    }
}

/// A Windows volume surfaced inside the distro, as the mount table reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrvfsMount {
    /// Where the volume is mounted inside the distro.
    pub mount_point: String,
    /// The Windows root it exposes: an upper-case drive letter, a colon and a backslash.
    pub windows_root: String,
}

/// The Windows root a mount source names, when it names one.
///
/// The mount source for a `DrvFS` mount is the drive it exposes; for anything else it is a
/// device node, a filesystem name, or a placeholder, and this returns `None` rather than
/// guessing.
#[must_use]
pub fn parse_drvfs_source(source: &str) -> Option<String> {
    let mut chars = source.chars();
    let drive = chars.next()?;
    if !drive.is_ascii_alphabetic() || chars.next()? != ':' {
        return None;
    }
    Some(format!("{}:\\", drive.to_ascii_uppercase()))
}

fn relative_under(path: &str, base: &str) -> Option<String> {
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        return Some(path.trim_start_matches('/').to_owned());
    }
    let rest = path.strip_prefix(base)?;
    if rest.is_empty() {
        return Some(String::new());
    }
    // Without this the prefix test would accept `/mnt/cd` as living under `/mnt/c`.
    Some(rest.strip_prefix('/')?.to_owned())
}

/// The Windows path a `DrvFS` path denotes, or `None` when the path is not under that mount.
#[must_use]
pub fn drvfs_to_windows(linux_path: &str, mount: &DrvfsMount) -> Option<String> {
    let rel = relative_under(linux_path, &mount.mount_point)?;
    let root = mount.windows_root.trim_end_matches('\\');
    if rel.is_empty() {
        Some(format!("{root}\\"))
    } else {
        Some(format!("{root}\\{}", rel.replace('/', "\\")))
    }
}

/// The `DrvFS` path a Windows path denotes, or `None` when the path is not on that volume.
/// Windows paths compare case-insensitively; the produced Linux path does not.
#[must_use]
pub fn windows_to_drvfs(windows_path: &str, mount: &DrvfsMount) -> Option<String> {
    let root = mount.windows_root.trim_end_matches('\\');
    let norm = windows_path.replace('/', "\\");
    if !norm
        .to_ascii_lowercase()
        .starts_with(&root.to_ascii_lowercase())
    {
        return None;
    }
    let rest = norm
        .get(root.len()..)?
        .trim_start_matches('\\')
        .replace('\\', "/");
    let base = mount.mount_point.trim_end_matches('/');
    let base = if base.is_empty() { "/" } else { base };
    if rest.is_empty() {
        Some(base.to_owned())
    } else {
        Some(format!("{base}/{rest}"))
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
    use super::{
        bridge_path, drvfs_to_windows, parse_bridge_path, parse_drvfs_source, windows_to_drvfs,
        BridgePath, DrvfsMount,
    };

    fn mnt_c() -> DrvfsMount {
        DrvfsMount {
            mount_point: "/mnt/c".to_owned(),
            windows_root: "C:\\".to_owned(),
        }
    }

    #[test]
    fn both_bridge_prefixes_parse_and_the_distro_keeps_its_case() {
        assert_eq!(
            parse_bridge_path(r"\\wsl$\alpha\home\u\p"),
            Some(BridgePath {
                distro: "alpha".to_owned(),
                linux_path: "/home/u/p".to_owned()
            })
        );
        assert_eq!(
            parse_bridge_path(r"\\WSL.LOCALHOST\beta\srv"),
            Some(BridgePath {
                distro: "beta".to_owned(),
                linux_path: "/srv".to_owned()
            })
        );
    }

    #[test]
    fn the_distro_root_is_the_linux_root() {
        assert_eq!(
            parse_bridge_path(r"\\wsl.localhost\gamma"),
            Some(BridgePath {
                distro: "gamma".to_owned(),
                linux_path: "/".to_owned()
            })
        );
    }

    #[test]
    fn a_path_that_is_not_on_the_bridge_is_not_translated() {
        assert_eq!(parse_bridge_path("/home/u/p"), None);
        assert_eq!(parse_bridge_path(r"C:\code\p"), None);
        assert_eq!(parse_bridge_path(r"\\fileserver\share\p"), None);
        assert_eq!(parse_bridge_path(r"\\wsl$\"), None);
    }

    #[test]
    fn the_bridge_form_round_trips() {
        let raw = r"\\wsl.localhost\alpha\home\u\p";
        let parsed = parse_bridge_path(raw).expect("parses");
        assert_eq!(bridge_path(&parsed.distro, &parsed.linux_path), raw);
        assert_eq!(bridge_path("alpha", "/"), r"\\wsl.localhost\alpha");
    }

    #[test]
    fn a_drvfs_source_names_its_windows_root_and_nothing_else_does() {
        assert_eq!(parse_drvfs_source("C:\\"), Some("C:\\".to_owned()));
        assert_eq!(parse_drvfs_source("d:"), Some("D:\\".to_owned()));
        assert_eq!(parse_drvfs_source("drvfs"), None);
        assert_eq!(parse_drvfs_source("/dev/sdc"), None);
    }

    #[test]
    fn the_drvfs_pair_round_trips() {
        let m = mnt_c();
        assert_eq!(
            drvfs_to_windows("/mnt/c/code/p", &m).as_deref(),
            Some("C:\\code\\p")
        );
        assert_eq!(drvfs_to_windows("/mnt/c", &m).as_deref(), Some("C:\\"));
        assert_eq!(
            windows_to_drvfs("C:\\code\\p", &m).as_deref(),
            Some("/mnt/c/code/p")
        );
        assert_eq!(
            windows_to_drvfs("c:/code/p", &m).as_deref(),
            Some("/mnt/c/code/p")
        );
        assert_eq!(windows_to_drvfs("C:\\", &m).as_deref(), Some("/mnt/c"));
    }

    #[test]
    fn a_sibling_directory_is_not_under_the_mount_point() {
        // The prefix test must land on a separator. `/mnt/cd` shares five characters with
        // `/mnt/c` and is a different filesystem.
        assert_eq!(drvfs_to_windows("/mnt/cd/p", &mnt_c()), None);
        assert_eq!(windows_to_drvfs("D:\\code", &mnt_c()), None);
    }
}

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! §4.5 has two clauses and this file pins both: the bridge is registered and never traversed,
//! and an in-distro filesystem is classified by **type**, never by matching `/mnt/[a-z]` by name.

use codotheca_core::scan::wsl::{is_drvfs_fstype, wsl_boundary};
use std::path::Path;

#[test]
fn both_bridge_prefixes_yield_the_distro() {
    assert_eq!(
        wsl_boundary(Path::new(r"\\wsl$\Ubuntu\home\u\p")).map(|b| b.distro),
        Some("Ubuntu".to_owned())
    );
    assert_eq!(
        wsl_boundary(Path::new(r"\\wsl.localhost\Alpine\srv\code")).map(|b| b.distro),
        Some("Alpine".to_owned())
    );
}

#[test]
fn the_unc_host_is_case_insensitive_but_the_distro_keeps_its_case() {
    assert_eq!(
        wsl_boundary(Path::new(r"\\WSL.LOCALHOST\Debian\home")).map(|b| b.distro),
        Some("Debian".to_owned())
    );
}

#[test]
fn the_distro_root_itself_is_a_boundary() {
    assert_eq!(
        wsl_boundary(Path::new(r"\\wsl$\Ubuntu")).map(|b| b.distro),
        Some("Ubuntu".to_owned())
    );
}

/// A bridge path spelled with forward slashes is the same bridge. Windows accepts both and a
/// root the user typed is not guaranteed to use backslashes.
#[test]
fn forward_slashes_name_the_same_bridge() {
    assert_eq!(
        wsl_boundary(Path::new("//wsl$/Ubuntu/home")).map(|b| b.distro),
        Some("Ubuntu".to_owned())
    );
}

#[test]
fn ordinary_and_other_unc_paths_are_not_boundaries() {
    assert!(wsl_boundary(Path::new("/home/u/p")).is_none());
    assert!(wsl_boundary(Path::new(r"C:\code\project")).is_none());
    assert!(wsl_boundary(Path::new(r"\\fileserver\share\p")).is_none());
    assert!(wsl_boundary(Path::new(r"\\wsl$\")).is_none());
}

#[test]
fn drvfs_is_decided_by_filesystem_type_never_by_a_mnt_prefix() {
    assert!(is_drvfs_fstype("9p"));
    assert!(is_drvfs_fstype("v9fs"));
    assert!(is_drvfs_fstype("drvfs"));
    assert!(is_drvfs_fstype("DrvFS"));
    assert!(!is_drvfs_fstype("ext4"));
    assert!(!is_drvfs_fstype("overlay"));
    // §4.5's explicit prohibition: a mount point is not a filesystem type. `/mnt/data` on ext4
    // is not a bridge, and an implementation that matched on the name would say it was.
    assert!(!is_drvfs_fstype("/mnt/c"));
    assert!(!is_drvfs_fstype("/mnt/data"));
}

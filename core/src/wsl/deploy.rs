//! §13 — installing the worker inside a distro.
//!
//! Running the ELF from the Windows mount would put every one of its own page faults across the
//! 9p bridge, which is the cost §4.5 exists to avoid — so it is copied in once per build. The
//! directory is named by the binary's own content, which makes "is this build deployed?" a
//! filesystem question, and makes the cleanup on version change exact.
//!
//! Phase 1 has no destructive operation. The one removal here is of a file Codotheca itself
//! wrote, at a path built from a name validated as a fingerprint, using `rmdir` — which refuses
//! a non-empty directory, so anything the app did not put there survives.

pub const WORKER_FILE_NAME: &str = "codotheca-worker";
pub const DEPLOY_ROOT_RELATIVE: &str = ".cache/codotheca/worker";

const FNV_OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
const FNV_PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

/// A 128-bit FNV-1a of the binary, in lowercase hex. This distinguishes Codotheca's own build
/// outputs from one another; it is not a security boundary and needs no cryptographic property.
#[must_use]
pub fn worker_fingerprint(bytes: &[u8]) -> String {
    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:032x}")
}

/// The only shape a directory name may have for this module to build a path from it.
#[must_use]
pub fn is_fingerprint_name(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployPaths {
    pub root: String,
    pub version_dir: String,
    pub exe: String,
    pub hooks_dir: String,
}

/// `None` when the home directory is not absolute or the fingerprint is not well formed — a
/// refusal, never a path assembled from an unvalidated component.
#[must_use]
pub fn deploy_paths(home: &str, fingerprint: &str) -> Option<DeployPaths> {
    if !home.starts_with('/') || !is_fingerprint_name(fingerprint) {
        return None;
    }
    let home = home.trim_end_matches('/');
    let root = format!("{home}/{DEPLOY_ROOT_RELATIVE}");
    let version_dir = format!("{root}/{fingerprint}");
    Some(DeployPaths {
        exe: format!("{version_dir}/{WORKER_FILE_NAME}"),
        hooks_dir: format!("{version_dir}/hooks"),
        version_dir,
        root,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployPlan {
    pub exe: String,
    pub install: bool,
    pub stale_dirs: Vec<String>,
}

/// What has to happen before the worker can be launched. `siblings` are the entry names directly
/// under `paths.root`; anything that is not a well-formed fingerprint is left alone entirely.
#[must_use]
pub fn plan_deploy(paths: &DeployPaths, exe_present: bool, siblings: &[String]) -> DeployPlan {
    let current = paths
        .version_dir
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned();
    let stale_dirs = siblings
        .iter()
        .filter(|name| is_fingerprint_name(name) && **name != current)
        .map(|name| format!("{}/{name}", paths.root))
        .collect();
    DeployPlan {
        exe: paths.exe.clone(),
        install: !exe_present,
        stale_dirs,
    }
}

/// `mkdir -p` creates parents and overwrites nothing that already exists.
#[must_use]
pub fn mkdir_argv(dir: &str) -> Vec<String> {
    vec!["mkdir".to_owned(), "-p".to_owned(), dir.to_owned()]
}

/// The bytes arrive on stdin. `tee` is in coreutils and in busybox, so it is present in both a
/// full and a minimal distro, and it needs no shell to redirect.
#[must_use]
pub fn write_argv(dest: &str) -> Vec<String> {
    vec!["tee".to_owned(), dest.to_owned()]
}

#[must_use]
pub fn chmod_argv(dest: &str) -> Vec<String> {
    vec!["chmod".to_owned(), "0755".to_owned(), dest.to_owned()]
}

#[must_use]
pub fn list_root_argv(root: &str) -> Vec<String> {
    vec!["ls".to_owned(), "-1".to_owned(), root.to_owned()]
}

/// Resolving `$HOME` without a shell: `getent passwd` is not needed because the launch already
/// fixes the user, and `printenv` reports that user's environment.
#[must_use]
pub fn home_argv() -> Vec<String> {
    vec!["printenv".to_owned(), "HOME".to_owned()]
}

/// The removals for one stale versioned directory, in order. Every directory removal is `rmdir`,
/// which fails on a non-empty directory: the bound on what can be removed is the tool's, not a
/// promise in a comment.
#[must_use]
pub fn cleanup_argvs(version_dir: &str) -> Vec<Vec<String>> {
    vec![
        vec![
            "rm".to_owned(),
            "-f".to_owned(),
            format!("{version_dir}/{WORKER_FILE_NAME}"),
        ],
        vec!["rmdir".to_owned(), format!("{version_dir}/hooks")],
        vec!["rmdir".to_owned(), version_dir.to_owned()],
    ]
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
        cleanup_argvs, deploy_paths, is_fingerprint_name, plan_deploy, worker_fingerprint,
        DeployPaths,
    };

    fn paths(fp: &str) -> DeployPaths {
        deploy_paths("/home/me", fp).expect("well formed")
    }

    #[test]
    fn the_fingerprint_is_content_addressed_and_well_formed() {
        let a = worker_fingerprint(b"one build");
        let b = worker_fingerprint(b"another build");
        assert_ne!(a, b);
        assert_eq!(a, worker_fingerprint(b"one build"));
        assert_eq!(a.len(), 32);
        assert!(is_fingerprint_name(&a));
    }

    #[test]
    fn a_name_that_is_not_a_fingerprint_is_refused() {
        for bad in [
            "",
            ".",
            "..",
            "../etc",
            "/etc",
            "~",
            "ABCDEF",
            "abc",
            &"a".repeat(33),
        ] {
            assert!(!is_fingerprint_name(bad), "{bad} must not pass");
        }
    }

    #[test]
    fn every_path_lives_under_the_versioned_directory() {
        let fp = worker_fingerprint(b"x");
        let p = paths(&fp);
        assert_eq!(p.root, "/home/me/.cache/codotheca/worker");
        assert_eq!(
            p.version_dir,
            format!("/home/me/.cache/codotheca/worker/{fp}")
        );
        assert_eq!(p.exe, format!("{}/codotheca-worker", p.version_dir));
        assert!(p.hooks_dir.starts_with(&p.version_dir));
    }

    #[test]
    fn a_traversing_fingerprint_yields_no_paths_at_all() {
        assert!(deploy_paths("/home/me", "../../etc").is_none());
        assert!(deploy_paths("/home/me", "").is_none());
        assert!(deploy_paths("", &worker_fingerprint(b"x")).is_none());
        assert!(deploy_paths("relative/home", &worker_fingerprint(b"x")).is_none());
    }

    #[test]
    fn an_already_deployed_worker_is_not_reinstalled() {
        let fp = worker_fingerprint(b"x");
        let plan = plan_deploy(&paths(&fp), true, std::slice::from_ref(&fp));
        assert!(!plan.install);
        assert!(plan.stale_dirs.is_empty());
    }

    #[test]
    fn a_version_change_installs_the_new_one_and_lists_only_old_ones() {
        let old = worker_fingerprint(b"old");
        let new = worker_fingerprint(b"new");
        let plan = plan_deploy(&paths(&new), false, &[old.clone(), new.clone()]);
        assert!(plan.install);
        assert_eq!(
            plan.stale_dirs,
            vec![format!("/home/me/.cache/codotheca/worker/{old}")]
        );
    }

    #[test]
    fn a_sibling_that_is_not_ours_is_never_named_in_a_removal() {
        let new = worker_fingerprint(b"new");
        let plan = plan_deploy(
            &paths(&new),
            false,
            &[
                "..".to_owned(),
                "notes.txt".to_owned(),
                "IMPORTANT".to_owned(),
                worker_fingerprint(b"old"),
            ],
        );
        assert_eq!(
            plan.stale_dirs.len(),
            1,
            "only the well-formed old fingerprint"
        );
        for named in &plan.stale_dirs {
            assert!(named.starts_with("/home/me/.cache/codotheca/worker/"));
            assert!(!named.contains(".."));
        }
    }

    #[test]
    fn cleanup_cannot_recurse_and_cannot_force_a_directory() {
        let argvs = cleanup_argvs("/home/me/.cache/codotheca/worker/abc");
        assert_eq!(
            argvs,
            vec![
                vec![
                    "rm".to_owned(),
                    "-f".to_owned(),
                    "/home/me/.cache/codotheca/worker/abc/codotheca-worker".to_owned()
                ],
                vec![
                    "rmdir".to_owned(),
                    "/home/me/.cache/codotheca/worker/abc/hooks".to_owned()
                ],
                vec![
                    "rmdir".to_owned(),
                    "/home/me/.cache/codotheca/worker/abc".to_owned()
                ],
            ]
        );
        // rmdir refuses a non-empty directory. That is the bound, enforced by the tool.
        for argv in &argvs {
            let is_dir_removal = argv.first().map(String::as_str) == Some("rmdir");
            if is_dir_removal {
                assert_eq!(argv.len(), 2, "a directory removal takes no flags");
            }
            assert!(!argv.iter().any(|a| a == "-r" || a == "-R" || a == "-rf"));
        }
    }

    #[test]
    fn the_install_argvs_run_no_shell() {
        for argv in [
            super::mkdir_argv("/d"),
            super::write_argv("/d/w"),
            super::chmod_argv("/d/w"),
            super::list_root_argv("/d"),
            super::home_argv(),
        ] {
            for banned in ["sh", "bash", "-c", "eval"] {
                assert!(!argv.iter().any(|a| a == banned), "{banned} in {argv:?}");
            }
        }
        assert_eq!(
            super::write_argv("/d/w"),
            vec!["tee".to_owned(), "/d/w".to_owned()]
        );
        assert_eq!(
            super::chmod_argv("/d/w"),
            vec!["chmod".to_owned(), "0755".to_owned(), "/d/w".to_owned()]
        );
    }
}

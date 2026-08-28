//! §4.2's last bullet and §4.4 — submodules.
//!
//! The walk stops at a repository root, so a submodule is never reached by descent. It is
//! enumerated from the superproject's `.gitmodules`, classified like any other directory, and
//! emitted as **its own repository** plus an edge. §4.4 is emphatic that it is not a location of
//! the parent: its lineage differs from the parent's, and one row holding both is the corruption
//! identity exists to prevent.
//!
//! Three rulings this module makes, because §4.2 states the mechanism and not the details:
//! `.gitmodules` is read under the 256 KB cap §10.1 promised the user for the four root files it
//! names; a path with a `..` component or an absolute path is rejected outright, because
//! `.gitmodules` is repository content and can say anything; and an uninitialised submodule
//! produces nothing, because there is no repository on disk for
//! `submodule_edge.child_project_id` (§1.9) to name.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use crate::index::path::native_platform;
use crate::paths::{path_display, path_from_bytes, path_key};
use crate::scan::discover::{classify_dir, ProbeCtx};
use crate::scan::{ScanProblem, ScanProblemKind, WalkEvent, WalkOptions, WalkSink};

/// §10.1's promise: four named files at a repository root, 256 KB each.
pub const GITMODULES_BYTE_CAP: u64 = 256 * 1024;
/// Submodule nesting deeper than this is a loop or an attack, not a project layout.
pub const MAX_SUBMODULE_DEPTH: usize = 32;

/// One row of `submodule_edge` (§1.9), minus the ids plan 08 assigns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmoduleEdgeCandidate {
    pub parent_worktree: PathBuf,
    pub child_worktree: PathBuf,
    /// The submodule's path within the parent, exactly as `.gitmodules` records it.
    pub path_bytes: Vec<u8>,
    /// The gitlink OID from the parent's index. `None` when the parent could not be read —
    /// **never zero and never a placeholder OID**, which would name a commit that does not exist.
    pub gitlink_oid: Option<String>,
}

/// `path =` values inside `[submodule "…"]` sections, in file order.
#[must_use]
pub fn parse_gitmodules(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut in_submodule = false;
    for raw in bytes.split(|b| *b == b'\n') {
        let line = trim_ascii(raw);
        if line.first() == Some(&b'[') {
            in_submodule = line.starts_with(b"[submodule");
            continue;
        }
        if !in_submodule {
            continue;
        }
        let Some(eq) = line.iter().position(|b| *b == b'=') else {
            continue;
        };
        let (key, rest) = line.split_at(eq);
        if trim_ascii(key) != b"path" {
            continue;
        }
        let value = trim_ascii(rest.get(1..).unwrap_or_default());
        if !value.is_empty() {
            out.push(value.to_vec());
        }
    }
    out
}

/// A submodule path is a path **within** the parent: relative, and made of normal components.
#[must_use]
pub fn is_safe_submodule_path(raw: &[u8]) -> bool {
    if raw.is_empty() {
        return false;
    }
    let path = path_from_bytes(raw);
    !path.is_absolute()
        && path.components().next().is_some()
        && path.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Enumerate this repository's submodules, recursively. Emits `WalkEvent::Repo` for each child
/// when `emit_repos` is set — which the walk clears when `descend_into_repos` would find them
/// anyway — and always emits `WalkEvent::SubmoduleEdge`.
pub fn enumerate_submodules(
    parent: &Path,
    opts: &WalkOptions,
    probe: &ProbeCtx<'_>,
    emit_repos: bool,
    sink: &WalkSink<'_>,
) -> Vec<SubmoduleEdgeCandidate> {
    let mut edges = Vec::new();
    let mut visited = HashSet::new();
    visited.insert(path_key(parent, native_platform()));
    let mut walk = SubmoduleWalk {
        opts,
        probe,
        emit_repos,
        sink,
        visited,
        out: &mut edges,
    };
    walk.descend(parent, 0);
    edges
}

/// The recursion's state, bundled so the walk is one argument rather than eight — `too_many_
/// arguments` fires at seven and an `allow` would hide a signature nobody can read anyway.
struct SubmoduleWalk<'a, 'p> {
    opts: &'a WalkOptions,
    probe: &'a ProbeCtx<'p>,
    emit_repos: bool,
    sink: &'a WalkSink<'a>,
    visited: HashSet<Vec<u8>>,
    out: &'a mut Vec<SubmoduleEdgeCandidate>,
}

impl SubmoduleWalk<'_, '_> {
    fn descend(&mut self, parent: &Path, depth: usize) {
        if depth >= MAX_SUBMODULE_DEPTH {
            return;
        }
        let Some(declared) = read_gitmodules(parent, self.sink) else {
            return;
        };
        let safe: Vec<Vec<u8>> = declared
            .into_iter()
            .filter(|raw| is_safe_submodule_path(raw))
            .collect();
        if safe.is_empty() {
            return;
        }
        let oids = self.probe.gitlinks(parent, &safe);

        for raw in safe {
            let child = parent.join(path_from_bytes(&raw));
            if !self.visited.insert(path_key(&child, native_platform())) {
                continue;
            }
            // A submodule is a repository in its own right; a `bare_candidates` probe has no
            // place here, so it is cleared regardless of the root's setting.
            let child_opts = WalkOptions {
                bare_candidates: false,
                ..*self.opts
            };
            let Some(candidate) = classify_dir(&child, &child_opts, self.probe, self.sink) else {
                continue;
            };
            let has_worktree = candidate.kind.has_worktree();
            if self.emit_repos {
                (self.sink)(WalkEvent::Repo(candidate));
            }
            let edge = SubmoduleEdgeCandidate {
                parent_worktree: parent.to_path_buf(),
                child_worktree: child.clone(),
                path_bytes: raw.clone(),
                gitlink_oid: oids.get(&raw).cloned(),
            };
            (self.sink)(WalkEvent::SubmoduleEdge(Box::new(edge.clone())));
            self.out.push(edge);
            if has_worktree {
                self.descend(&child, depth + 1);
            }
        }
    }
}

fn read_gitmodules(parent: &Path, sink: &WalkSink<'_>) -> Option<Vec<Vec<u8>>> {
    let file = parent.join(".gitmodules");
    let meta = std::fs::metadata(&file).ok()?;
    if meta.len() > GITMODULES_BYTE_CAP {
        sink(WalkEvent::Problem(ScanProblem {
            kind: ScanProblemKind::UnreadableRepo,
            path_display: path_display(&file),
            detail: format!(
                ".gitmodules is {} bytes; the cap is {GITMODULES_BYTE_CAP}",
                meta.len()
            ),
        }));
        return None;
    }
    let bytes = std::fs::read(&file).ok()?;
    Some(parse_gitmodules(&bytes))
}

fn trim_ascii(mut raw: &[u8]) -> &[u8] {
    while raw.first().is_some_and(u8::is_ascii_whitespace) {
        raw = raw.get(1..).unwrap_or_default();
    }
    while raw.last().is_some_and(u8::is_ascii_whitespace) {
        raw = raw.get(..raw.len().saturating_sub(1)).unwrap_or_default();
    }
    raw
}

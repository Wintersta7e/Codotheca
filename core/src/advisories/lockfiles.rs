//! §32.6's lockfile read: **six filenames, depth ≤ 3, worktree basis, total or `not_read`.**

/// The largest lockfile this read will open.
///
/// **J6's own 256 KB is unchanged** and still governs J6's named-file reads; this read carries its
/// own because 256 KB is refuted by a measurement on this very tree — its `package-lock.json` is
/// **287,417 bytes**, which J6's cap would have truncated. A lockfile is machine-generated and
/// grows with the dependency graph, not with anything a human wrote.
pub const LOCKFILE_BYTE_CAP: u64 = 16 * 1024 * 1024;

/// The largest number of lockfiles this read will open for one project.
///
/// **Two bounds and not three**: each file is parsed and its triples written before the next is
/// opened, so the peak memory is one file and [`LOCKFILE_BYTE_CAP`] already governs it. A third
/// number would be a third thing to drift.
pub const LOCKFILE_COUNT_CAP: usize = 32;

/// How far below the repository root the walk descends.
///
/// **Three and not one.** A read restricted to the repository root misses a monorepo's per-package
/// lockfiles and produces a **partial triple set**, whose verdict is a lit tick claiming *no known
/// vulnerable dependencies* over a read that never looked.
pub const LOCKFILE_MAX_DEPTH: usize = 3;

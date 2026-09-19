/**
 * [p3] §32.6's six lock file names, in **one** place the copy and its tests both read.
 *
 * The core owns the read; this list exists so the two rendered sites that must name the files —
 * §10.1's consent paragraph and §11.3's settings row — and the tests that check them cannot drift
 * from each other. It is deliberately **not** a second source of truth for the read itself:
 * `core/src/advisories/lockfiles.rs` holds `LOCKFILE_NAMES`, and nothing here is sent to it.
 *
 * The depth and the cap live beside the names because a sentence that names the files and not the
 * bound is a sentence that under-promises the read.
 */
export const LOCKFILE_NAMES = [
  'package-lock.json',
  'yarn.lock',
  'pnpm-lock.yaml',
  'Cargo.lock',
  'poetry.lock',
  'uv.lock',
] as const;

/** How far below the repository root the read looks, as the copy says it. */
export const LOCKFILE_DEPTH_TEXT = 'three directories deep';

/** The read's own per-file cap. **Not J6's 256 KB**, which this list does not govern. */
export const LOCKFILE_CAP_TEXT = '16 MB each';

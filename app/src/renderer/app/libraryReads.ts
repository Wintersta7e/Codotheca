/**
 * GA39: the core's bare `projects.list` drops every hidden, reference and excluded row
 * (`in_base_set`), while the renderer's evaluator applies that base predicate itself and so must
 * hold every row, or `is:hidden` after a reload finds only rows changed this session. Flag terms
 * AND-combine and a row is dropped for any flag the query does not name, so each read answers the
 * rows whose flags are exactly the ones it names — the eight subsets partition the library.
 * The core's `projects` topic snapshot makes the same reads (`PROJECTS_SNAPSHOT_READS`,
 * `core/src/assembly/mod.rs`); `app/test/libraryReads.test.ts` holds the two lists equal.
 */
export const LIBRARY_READS: readonly (string | null)[] = [
  null,
  'is:hidden',
  'is:reference',
  'is:excluded',
  'is:hidden is:reference',
  'is:hidden is:excluded',
  'is:reference is:excluded',
  'is:hidden is:reference is:excluded',
];

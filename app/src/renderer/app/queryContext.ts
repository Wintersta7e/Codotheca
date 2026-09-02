/**
 * One `QueryContext`, built in one place.
 *
 * The shelf evaluates the query with it and §10.4a's turn counts its rungs with it. Two
 * constructions is two readings of the same library — the shape R12 exists to prevent — so both
 * call sites come here.
 */
import type { QueryContext } from '../shelf/evaluate.js';
import { projectionCapabilities, type ShelfRow } from '../shelf/row.js';

/**
 * §8.3's `path:` terms fold case on Windows and not elsewhere. The core knows which
 * (`cfg!(not(windows))` at `projects/list.rs`) and **the wire does not carry it**, so the
 * renderer has no reading of its own. `false` is the narrower answer — it matches fewer rows
 * rather than more — and the gap is recorded rather than guessed at per call site.
 */
export const PATHS_ARE_CASE_SENSITIVE = false;

export interface QueryContextInput {
  readonly rows: readonly ShelfRow[];
  /** Unix seconds. */
  readonly now: number;
  readonly firstRunCompletedAt: number | null;
  /**
   * §8.8's saved queries. Empty until something mounts `CollectionsProvider`, which is 15b's and
   * is not mounted here — a `collection:<name>` term therefore resolves against nothing, which
   * `evaluateQuery` answers as *unknown* rather than as *no match*.
   */
  readonly collectionIdsByName?: ReadonlyMap<string, number>;
}

const NO_COLLECTIONS: ReadonlyMap<string, number> = new Map();

export function buildQueryContext(input: QueryContextInput): QueryContext {
  return {
    now: input.now,
    firstRunCompletedAt: input.firstRunCompletedAt,
    collectionIdsByName: input.collectionIdsByName ?? NO_COLLECTIONS,
    pathsAreCaseSensitive: PATHS_ARE_CASE_SENSITIVE,
    capabilities: projectionCapabilities(input.rows),
    // §8.3's commit-subject lane is a debounced search this mount does not run. `null` is *no
    // search in flight*, which the evaluator reads as unknown rather than as no hits.
    commitSubjectHits: null,
  };
}

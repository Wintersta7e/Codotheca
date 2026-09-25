import type { LocationRef, ProjectId, ProjectRow } from '../../generated/protocol.js';
import { type ShelfRow, toShelfRow } from '../shelf/row.js';

/**
 * A projection row with every field at its most boring value, so a test states only the two or
 * three fields it is about. Imported by tests only; nothing in the bundle reaches it.
 *
 * **`primaryLocation` and `presence` are one pair and are set together.** This builder used to
 * pair a null location with `'present'` — a working copy that is here, for a row that has no
 * copy at all — and after §23.4's classifier that default would file every fixture in the tree
 * under `era:notcloned`. A default that silently satisfies the predicate under test is the same
 * defect as a gate that scans zero files, so the zero-location row is `notClonedRow` and a test
 * has to ask for it by name.
 */
export function makeProjectRow(overrides: Partial<ProjectRow> = {}): ProjectRow {
  const base: ProjectRow = {
    id: 1 as ProjectId,
    name: 'row',
    owner: null,
    description: null,
    descriptionSource: null,
    birthYear: null,
    primaryLanguage: null,
    archetype: null,
    seedBasename: 'row',
    rerollOffset: 0,
    artSceneHash: null,
    artState: 'pending',
    conditionSignal: null,
    completionLit: null,
    completionApplicable: null,
    isPinned: false,
    isArchived: false,
    isHidden: false,
    isReference: false,
    // J1.5 has not run: null, never `false`, which would say someone else's.
    authoredByUser: null,
    isFork: false,
    isBare: false,
    isShallow: false,
    isSubmodule: false,
    ambiguousLineage: false,
    lastTouchedAt: 1_700_000_000,
    lastInteractionAt: null,
    lastCommitAt: null,
    lastCommitSubject: null,
    firstCommitAt: null,
    createdAt: 1_700_000_000,
    acknowledgedAt: null,
    sizeTrackedBytes: null,
    trackedFiles: null,
    collectionIds: [],
    primaryLocation: { id: 10 as LocationRef['id'], pathDisplay: '/w/row' },
    presence: 'present',
    hasRemote: false,
    branch: null,
    isDirty: null,
    untrackedCount: null,
    ahead: null,
    behind: null,
    stashCount: null,
    interruptedOp: null,
    fetchHeadAt: null,
    refstateObservedAt: null,
    worktreeObservedAt: null,
    errorKind: null,
    errorAt: null,
    eraSectionId: 'era:live',
    // [p3] §30.1's most boring reading is the one that says nothing was computed: `absent`, with
    // every quantity null. A fixture defaulting to `live` with a zero count would satisfy the
    // *nothing outstanding* predicate for every test that did not ask about health.
    healthSummary: {
      state: 'absent',
      scoredOpen: null,
      unverified: null,
      unknownChecks: null,
      observedAt: null,
    },
    lifecycle: 'active',
  };
  return { ...base, ...overrides } satisfies ProjectRow;
}

export function makeLocationRef(id: number, pathDisplay = '/w/row'): LocationRef {
  return { id: id as LocationRef['id'], pathDisplay };
}

/**
 * §23.1's shape: a project Codotheca knows of and holds **no working copy of**. One row, zero
 * locations — so `primaryLocation` is `null` and `presence` is `null` with it. The pair is the
 * whole predicate; `presence IS NULL` is the same predicate rendered, not a second source.
 *
 * Returns a `ShelfRow` because the surfaces that render one — the query evaluator, the card, the
 * palette — take the projection with its extras, and `ShelfRow` is assignable wherever a
 * `ProjectRow` is wanted.
 */
export function notClonedRow(overrides: Partial<ShelfRow> = {}): ShelfRow {
  return {
    ...toShelfRow(makeProjectRow({ primaryLocation: null, presence: null })),
    ...overrides,
  };
}

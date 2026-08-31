import type { LocationRef, ProjectId, ProjectRow } from '../../generated/protocol.js';

/**
 * A projection row with every field at its most boring value, so a test states only the two or
 * three fields it is about. Imported by tests only; nothing in the bundle reaches it.
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
    primaryLocation: null,
    presence: 'present',
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
  };
  return { ...base, ...overrides } satisfies ProjectRow;
}

export function makeLocationRef(id: number, pathDisplay = '/w/row'): LocationRef {
  return { id: id as LocationRef['id'], pathDisplay };
}

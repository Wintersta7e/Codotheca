import { describe, expect, it, vi } from 'vitest';
import type {
  HealthState,
  HealthSummary,
  LocationRef,
  ProjectId,
  ProjectRow,
} from '../../generated/protocol.js';
import { ProjectionStore } from './projection.js';
import { projectionCapabilities, toShelfRow } from './row.js';

/** Plan 02 brands every id, so a test that names one has to say which brand it means. */
const pid = (n: number): ProjectId => n as ProjectId;

function row(id: number, over: Partial<ProjectRow> = {}): ProjectRow {
  return {
    id,
    name: `p${id}`,
    owner: null,
    description: null,
    descriptionSource: null,
    birthYear: null,
    primaryLanguage: null,
    archetype: null,
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
    lastTouchedAt: 0,
    lastInteractionAt: null,
    lastCommitAt: null,
    lastCommitSubject: null,
    firstCommitAt: null,
    createdAt: 0,
    acknowledgedAt: null,
    sizeTrackedBytes: null,
    trackedFiles: null,
    collectionIds: [],
    // §23: location and presence are one pair — a null location beside 'present'
    // describes a state the product cannot produce, and §23.4's classifier files
    // every such row under era:notcloned.
    primaryLocation: { id: 10 as LocationRef['id'], pathDisplay: '/w/row' },
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
    eraSectionId: '',
    // [p3] `healthSummary` is non-nullable (R116), and §30.1's most boring reading is the one
    // that says nothing was computed. A fixture defaulting to `live` with a zero count would
    // answer the health capability for every test that never asked about health.
    healthSummary: {
      state: 'absent',
      scoredOpen: null,
      unverified: null,
      unknownChecks: null,
      observedAt: null,
    },
    lifecycle: 'active',
    ...over,
  } as unknown as ProjectRow;
}

/** A reading, for the rows a health test is about. */
function reading(state: HealthState, scoredOpen: number | null = null): HealthSummary {
  return { state, scoredOpen, unverified: null, unknownChecks: null, observedAt: null };
}

describe('toShelfRow', () => {
  it('fills every extra with null when the wire row does not carry it', () => {
    const shelfRow = toShelfRow(row(1));
    expect(shelfRow.authoredByUser).toBeNull();
    expect(shelfRow.locationKind).toBeNull();
    expect(shelfRow.hasCi).toBeNull();
  });
  it('picks an extra up when the wire row does carry it', () => {
    const shelfRow = toShelfRow({ ...row(1), authoredByUser: true } as unknown as ProjectRow);
    expect(shelfRow.authoredByUser).toBe(true);
  });
});

describe('projectionCapabilities', () => {
  it('reports a capability as absent when no row answers it', () => {
    expect(projectionCapabilities([toShelfRow(row(1))]).authoredByUser).toBe(false);
  });
  it('reports it present as soon as one row answers it', () => {
    const rows = [
      toShelfRow(row(1)),
      toShelfRow({ ...row(2), authoredByUser: false } as unknown as ProjectRow),
    ];
    expect(projectionCapabilities(rows).authoredByUser).toBe(true);
  });

  // [p3] §35.5's ninth capability, on the same rule as the eight above it.
  it('AC-P3-35-6 answers health only when a row carries a reading', () => {
    const caps = (over: Partial<ProjectRow>): boolean =>
      projectionCapabilities([toShelfRow(row(1, over))]).health;

    // An empty projection answers nothing.
    expect(projectionCapabilities([]).health).toBe(false);
    // `absent` and `suppressed` are the two states that carry no reading.
    expect(caps({})).toBe(false);
    expect(caps({ healthSummary: reading('suppressed') })).toBe(false);
    // A computed zero **answers it**: the project carries a reading and its count is zero.
    expect(caps({ healthSummary: reading('live', 0) })).toBe(true);
    expect(caps({ healthSummary: reading('frozen', 4) })).toBe(true);
    // One row is enough, exactly as for the eight above.
    const mixed = [toShelfRow(row(1)), toShelfRow(row(2, { healthSummary: reading('live', 2) }))];
    expect(projectionCapabilities(mixed).health).toBe(true);
  });
});

describe('ProjectionStore', () => {
  it('replaces everything on a snapshot and records the generation', () => {
    const store = new ProjectionStore();
    store.applySnapshot([row(1), row(2)], 7);
    store.applySnapshot([row(3)], 8);
    expect(store.rows.map((r) => r.id)).toEqual([3]);
    expect(store.generation).toBe(8);
  });
  it('upserts in place without reordering the existing rows', () => {
    const store = new ProjectionStore();
    store.applySnapshot([row(1), row(2)], 1);
    store.upsert(row(1, { name: 'renamed' }));
    expect(store.rows.map((r) => r.name)).toEqual(['renamed', 'p2']);
  });
  it('appends an unseen row', () => {
    const store = new ProjectionStore();
    store.applySnapshot([row(1)], 1);
    store.upsert(row(2));
    expect(store.rows.map((r) => r.id)).toEqual([1, 2]);
  });
  it('collapses two visible tiles into one on merge', () => {
    // §2.4: projects.merged did not exist in v1, so the renderer had no way to do this.
    const store = new ProjectionStore();
    store.applySnapshot([row(1), row(2)], 1);
    store.merge(pid(1), pid(2));
    expect(store.rows.map((r) => r.id)).toEqual([2]);
  });
  it('applies a flag delta without touching anything else', () => {
    const store = new ProjectionStore();
    store.applySnapshot([row(1)], 1);
    store.setFlags(pid(1), { isPinned: true, isArchived: false, isHidden: false });
    expect(store.get(pid(1))?.isPinned).toBe(true);
    expect(store.get(pid(1))?.name).toBe('p1');
  });
  it('applies a condition delta, including back to null', () => {
    const store = new ProjectionStore();
    store.applySnapshot([row(1, { conditionSignal: 'live' })], 1);
    store.setCondition(pid(1), null);
    expect(store.get(pid(1))?.conditionSignal).toBeNull();
  });
  it('notifies subscribers once per mutation and stops on unsubscribe', () => {
    const store = new ProjectionStore();
    const seen = vi.fn();
    const off = store.subscribe(seen);
    store.applySnapshot([row(1)], 1);
    store.upsert(row(2));
    off();
    store.upsert(row(3));
    expect(seen).toHaveBeenCalledTimes(2);
  });
  it('ignores a delta for an id it has never seen, rather than inventing a partial row', () => {
    const store = new ProjectionStore();
    store.applySnapshot([row(1)], 1);
    store.setCondition(pid(99), 'live');
    expect(store.rows).toHaveLength(1);
  });
});

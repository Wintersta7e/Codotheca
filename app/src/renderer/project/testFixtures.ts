/**
 * One fixture builder for the whole page. Values are shaped, never named: no real project, no
 * real path, no real user.
 *
 * The defaults are a project that has been fully observed, because most of the page's tests are
 * about what it draws from real values. The phase-1 default of *nothing computed* is reached by
 * overriding, so a test that means "uncomputed" says so out loud.
 */
import type {
  Activity,
  CommitRef,
  LocationDetail,
  LocationId,
  ProjectDetail,
  ProjectId,
  ProjectRow,
  RemoteFacts,
  SceneHash,
  TargetId,
  TargetRow,
} from '../../generated/protocol';

export const NOW = 1_800_000_000;
export const DAY = 86_400;

/**
 * §25.1's facts, defaulted to *nothing observed* — which is what every project reads until
 * §21's runner lands in wave 5. A test that is about an observed value says so by overriding,
 * so a fixture never quietly asserts a number the product cannot have produced yet.
 */
export function remoteFactsFixture(over: Partial<RemoteFacts> = {}): RemoteFacts {
  return {
    key: 'github.com/acme/widget',
    linkable: true,
    state: 'not_observed',
    visibility: null,
    forkParentKey: null,
    stars: null,
    openIssues: null,
    goodFirstIssues: null,
    openPrs: null,
    openPrsFromUser: null,
    topics: [],
    observedAt: null,
    ci: { state: 'not_observed', runs: [], observedAt: null },
    ...over,
  };
}

let nextLocationId = 1;

export function locationFixture(over: Partial<LocationDetail> = {}): LocationDetail {
  const id = nextLocationId++ as unknown as LocationId;
  return {
    location: { id, pathDisplay: '~/work/aurora' },
    kind: 'linux',
    distro: '',
    presence: 'present',
    isPrimary: true,
    branch: 'main',
    headOid: 'a1b2c3d4e5f6',
    headComparison: 'same_commit',
    ahead: 0,
    behind: 0,
    isDirty: false,
    untrackedCount: 0,
    stashCount: 0,
    interruptedOp: null,
    lastSeenAt: NOW - 60,
    refstateObservedAt: NOW - 60,
    worktreeObservedAt: NOW - 60,
    fetchHeadAt: NOW - 6 * DAY,
    trustedAt: null,
    coveringRootId: null,
    // [p2] §24.6a: NULL is *not removed*. A location that was uninstalled carries the instant,
    // and that instant takes precedence over `presence` on every surface.
    removedAt: null,
    ...over,
  };
}

export function rowFixture(over: Partial<ProjectRow> = {}): ProjectRow {
  return {
    id: 7 as unknown as ProjectId,
    name: 'aurora',
    owner: null,
    description: 'A shaped description, from the manifest.',
    descriptionSource: 'manifest',
    birthYear: 2019,
    primaryLanguage: 'Rust',
    archetype: 'cli',
    seedBasename: 'aurora',
    rerollOffset: 0,
    artSceneHash: 'aa11bb22' as unknown as SceneHash,
    artState: 'ready',
    conditionSignal: 'idle',
    // Phase 1 computes neither, on every project. Nothing here may render a zero for them.
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
    lastTouchedAt: NOW - DAY,
    lastInteractionAt: NOW - DAY,
    lastCommitAt: NOW - DAY,
    lastCommitSubject: 'a shaped subject',
    firstCommitAt: NOW - 900 * DAY,
    createdAt: NOW - 900 * DAY,
    acknowledgedAt: NOW - 900 * DAY,
    sizeTrackedBytes: 8_400_000,
    trackedFiles: 412,
    collectionIds: [],
    primaryLocation: { id: 1 as unknown as LocationId, pathDisplay: '~/work/aurora' },
    presence: 'present',
    hasRemote: true,
    branch: 'main',
    isDirty: false,
    untrackedCount: 0,
    ahead: 0,
    behind: 0,
    stashCount: 0,
    interruptedOp: null,
    fetchHeadAt: NOW - 6 * DAY,
    refstateObservedAt: NOW - 60,
    worktreeObservedAt: NOW - 60,
    errorKind: null,
    errorAt: null,
    eraSectionId: 'era-2019',
    // [p3] §30.1: `absent` is the reading nothing has computed, and every quantity is null.
    healthSummary: {
      state: 'absent',
      scoredOpen: null,
      unverified: null,
      unknownChecks: null,
      observedAt: null,
    },
    lifecycle: 'active',
    ...over,
  };
}

export function targetFixture(over: Partial<TargetRow> = {}): TargetRow {
  return {
    id: 1 as unknown as TargetId,
    kind: 'editor',
    name: 'Editor A',
    projectId: null,
    locationId: null,
    language: null,
    sortIndex: 0,
    detected: true,
    verifyState: 'ok',
    verifiedAt: NOW - 3600,
    execDisplay: 'editor-a',
    ...over,
  };
}

export function activityFixture(over: Partial<Activity> = {}): Activity {
  const weeks = Array.from({ length: 26 }, (_unused, i) => ({
    weekStart: NOW - (25 - i) * 7 * DAY,
    commitDays: i % 4,
    sessionCount: 0,
    sessionSeconds: 0,
  }));
  return { weeks, commitDays: 'measured', sessions: 'measured', ...over };
}

export function commitFixture(over: Partial<CommitRef> = {}): CommitRef {
  return {
    sha: 'a1b2c3d4e5f6',
    subject: 'a shaped subject',
    at: NOW - DAY,
    tzOffsetMin: 0,
    ...over,
  };
}

export function detailFixture(over: Partial<ProjectDetail> = {}): ProjectDetail {
  nextLocationId = 1;
  const editor = targetFixture();
  return {
    row: rowFixture(),
    locations: [locationFixture()],
    resolvedTarget: { target: editor, tier: 'global' },
    targets: [
      editor,
      targetFixture({
        id: 2 as unknown as TargetId,
        kind: 'terminal',
        name: 'Shell',
        sortIndex: 0,
      }),
    ],
    readme: { state: 'present', text: 'A shaped README paragraph.', readAt: NOW - 3600 },
    notes: null,
    remoteKey: null,
    lineageKey: 'lineage-a',
    associationKind: null,
    seedBasename: 'aurora',
    rerollOffset: 0,
    playtimeSeconds: 0,
    liveSession: null,
    activity: activityFixture(),
    recentCommits: [commitFixture()],
    firstCommitSha: 'a1b2c3d4e5f6',
    firstCommitTzOffsetMin: 0,
    sizeWorktreeBytes: 12_000_000,
    // Null is "not yet resolved", which is what every project carries until a listing or a
    // lookup binds one — not a third state and not either variant.
    remoteLinkBasis: null,
    // §25.1: NULL **iff** `remote_key` is NULL, and `remoteKey` above is null — so the default
    // fixture is a project with no remote and two tabs. A test about the REMOTE tab overrides
    // both, which is what keeps the pair from drifting into a shape the core cannot produce.
    remote: null,
    // §25.3's row 1 follows from that same null `remoteKey`: a project with no remote is the
    // only copy on earth, whatever its working tree says. A fixture answering anything else
    // here would be a shape the producer cannot make.
    backup: 'only_copy',
    // [p3] §28.9: one flat list, grouped by layer in the renderer (R120). Empty is *this project
    // has no items*, which is a different fact from `debtSweeps` being empty — that one is
    // *nothing has been swept*, and a test about either overrides both.
    debt: [],
    debtSweeps: [],
    // [p3] §30.1. `absent` with an **empty** `checks`, which is the state saying nothing was
    // computed and never a count of zero checks. A test about the reading overrides it.
    health: { state: 'absent', scoredOpen: null, basis: null, checks: [] },
    ...over,
  };
}

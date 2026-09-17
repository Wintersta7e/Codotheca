import test from 'node:test';
import assert from 'node:assert/strict';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadSchema, parseTypeExpr } from '../lib/schema.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const schema = loadSchema(join(here, '../schema/protocol.json'));
const names = schema.commands.map((c) => c.name);

// §2.4's table, transcribed. This list is the conformance corpus: a command dropped or renamed
// fails here rather than being discovered by a renderer that cannot call it.
const INFRASTRUCTURE = [
  'app.hello_ack',
  'app.shutdown',
  'roots.suggest',
  'roots.list',
  'roots.add',
  'roots.remove',
  'roots.setEnabled',
  'roots.setDescend',
  'scan.start',
  'scan.cancel',
  'scan.status',
  'problems.list',
  'settings.get',
  'settings.set',
  'view.get',
  'view.set',
  'diag.bundle',
];

test('every infrastructure command of §2.4 is declared', () => {
  for (const n of INFRASTRUCTURE) assert.ok(names.includes(n), `${n} is missing`);
});

// [p2] §20.8 adds the two identity errors. Neither overloads PERMISSION_DENIED, which is a
// filesystem error — a shared name is not a shared shape.
test('the error enum is closed: §2.4 plus §1.6 PROJECT_MERGED plus §20.8 and nothing else', () => {
  assert.deepEqual([...schema.errors].sort(), [
    'BUDGET_EXCEEDED',
    'CORE_RESTARTED',
    'GIT_MISSING',
    'GIT_TOO_OLD',
    'INTERNAL',
    'PATH_GONE',
    'PERMISSION_DENIED',
    'PROJECT_MERGED',
    'PROTOCOL',
    'REPO_UNREADABLE',
    'SSO_REQUIRED',
    'STORE_OFFLINE',
    'TOKEN_INVALID',
    'UNTRUSTED_REPO',
  ]);
});

test('roots.add is the privileged shell-dialog path, and carries bytes not a string', () => {
  const c = schema.commands.find((x) => x.name === 'roots.add');
  assert.equal(c.privileged, true);
  assert.equal(c.args.pathBytes, 'Bytes');
});

// §11.3a draws one row per root and an `N OF M ACTIVE` caption over them. §2.4's table declared
// only the four mutations, so nothing in the contract returned the set the surface enumerates.
test('roots.list returns the whole set, and reads rather than mutates', () => {
  const c = schema.commands.find((x) => x.name === 'roots.list');
  assert.deepEqual(c.args, {});
  assert.equal(c.returns, '[Root]');
  assert.notEqual(c.privileged, true);
  assert.notEqual(c.idempotent, false);
});

// §11.1 + criterion 63: an offline row renders `last seen <age>` and the branch as last observed,
// and names no drive and no volume. The time is `location.last_seen_at`; NULL is never observed,
// which draws no age slot at all rather than an epoch date.
test('a problem item carries when its location was last observed', () => {
  assert.equal(schema.types.ProblemItem.fields.lastSeenAt, 'Timestamp?');
});

// §11.1's ambiguous row reads `Same history as <name> and <name>.` The wire carried ids and no
// names, and no command maps an id to a name, so the sentence was unrenderable. The core holds
// both and returns the first two names in the same order as the ids; the id array's length is
// what the `and <n> more` tail counts, which is why both fields are needed and neither is a
// substitute for the other.
test('an ambiguous row carries the names it needs to render its sentence', () => {
  assert.equal(schema.types.ProblemItem.fields.candidateNames, '[String]');
  assert.equal(schema.types.ProblemItem.fields.candidateProjectIds, '[ProjectId]');
});

// §11.1: while a scan is in flight the problem clause is omitted, never zeroed — a scan that has
// not finished has not yet found no problems.
test('scan and problem counts are nullable so an unfinished run cannot report zero problems', () => {
  assert.equal(schema.types.ScanStatus.fields.problemCount, 'u32?');
  assert.equal(schema.types.ScanStatus.fields.ambiguousLineageCount, 'u32?');
  assert.equal(schema.types.ProblemHeader.fields.problemCount, 'u32?');
});

// §10.1a: the count is provenance hits, not repositories, and reads `—` where no source named
// the row. Never `0`.
test('a root suggestion may have no hit count at all', () => {
  assert.equal(schema.types.RootSuggestion.fields.hits, 'u32?');
});

// §8.0a / §11.3a: Completion is dropped from the sort control — a key over an uncomputed column
// orders by unknown.
test('the sort key set excludes completion', () => {
  assert.deepEqual(schema.types.SortKey.variants, ['last_touched', 'name', 'size']);
});

test('§17: no command name carries a destructive verb, and FORGET appears nowhere', () => {
  const banned = /(forget|delete|uninstall|clean|push|checkout|prune|discard|reset)/i;
  for (const n of names) assert.doesNotMatch(n, banned, `${n} names a destructive operation`);
  assert.doesNotMatch(JSON.stringify(schema), /FORGET/);
});

const PROJECTS = [
  'projects.list',
  'projects.get',
  'projects.peek',
  'projects.setFlags',
  'projects.setNote',
  'projects.merge',
  'projects.unmergeHint',
  'projects.requeue',
  'projects.launch',
  'locations.setTrusted',
  'locations.relocate',
];

test('every project and location command of §2.4 is declared', () => {
  for (const n of PROJECTS) assert.ok(names.includes(n), `${n} is missing`);
});

test('projects.merge names symmetric inputs without promising a survivor', () => {
  const c = schema.commands.find((x) => x.name === 'projects.merge');
  assert.deepEqual(c.args, { a: 'ProjectId', b: 'ProjectId' });
});

// §2.4: v1 omitted locationId and could not express which copy of a multi-location project to
// open. §4bis.5: the renderer sends {projectId, locationId, targetId} and nothing else.
test('projects.launch carries all three ids and no path', () => {
  const c = schema.commands.find((x) => x.name === 'projects.launch');
  assert.deepEqual(c.args, {
    projectId: 'ProjectId',
    locationId: 'LocationId',
    targetId: 'TargetId',
  });
});

// §1.10 / §7.7a: nothing in phase 1 writes completion, and NULL is not 0.
test('completion crosses as nullable and is never a plain integer', () => {
  assert.equal(schema.types.ProjectRow.fields.completionLit, 'u32?');
  assert.equal(schema.types.ProjectRow.fields.completionApplicable, 'u32?');
});

// §6: absence of dirty means "no changes as of T", never "clean". A bool alone cannot say that.
test('worktree state crosses with its observation time, and is nullable', () => {
  assert.equal(schema.types.WorktreeObservation.fields.observedAt, 'Timestamp?');
  assert.equal(schema.types.WorktreeObservation.fields.isDirty, 'bool?');
  assert.equal(schema.types.ProjectRow.fields.isDirty, 'bool?');
  assert.equal(schema.types.ProjectRow.fields.worktreeObservedAt, 'Timestamp?');
});

// §1.2: condition_signal is NULL until a scan job has produced one, and that state is neither
// `empty` nor `offline`.
test('conditionSignal is nullable and carries exactly §5.4 vocabulary', () => {
  assert.equal(schema.types.ProjectRow.fields.conditionSignal, 'ConditionSignal?');
  assert.deepEqual(schema.types.ConditionSignal.variants, [
    'live',
    'idle',
    'dormant',
    'neglected',
    'abandoned',
    'offline',
    'empty',
  ]);
});

// §1.3 [v2.2]: fetch_head_at is the sole source of the BEHIND chip's age; NULL means no fetch has
// ever been recorded, rendered `no fetch recorded`, never as `0` and never as an age.
test('fetchHeadAt is nullable and separate from the observation clocks', () => {
  assert.equal(schema.types.ProjectRow.fields.fetchHeadAt, 'Timestamp?');
  assert.equal(schema.types.ProjectRow.fields.refstateObservedAt, 'Timestamp?');
});

// §8.2 [v2.2]: agg carries every figure §8.1's header draws — v2.1 shipped three of six — and
// `label` is gone, because section labels are user-facing prose owned by the shell.
test('era sections carry the six aggregates, the cut year and no label', () => {
  assert.deepEqual(Object.keys(schema.types.EraAggregate.fields).sort(), [
    'indexedCount',
    'interrupted',
    'trackedBytes',
    'unchecked',
    'uncommitted',
    'unpushed',
  ]);
  assert.ok(!('label' in schema.types.EraSection.fields));
  assert.equal(schema.types.EraSection.fields.cutAgainstYear, 'u32');
  assert.equal(schema.types.EraSection.fields.year, 'u32?');
});

// §8.5.5: 26 zero-height bars is the canonical picture of not-computed. A lane needs a state.
test('activity lanes carry a state so an uncomputed lane cannot render as zero', () => {
  assert.equal(schema.types.Activity.fields.commitDays, 'LaneState');
  assert.equal(schema.types.Activity.fields.sessions, 'LaneState');
  assert.equal(schema.types.ActivityWeek.fields.commitDays, 'u32?');
});

// §1.7 / §8.5.5: J4 produces commit-days; nothing stores a commit count.
//
// The rule is about counting, not about the word: `Peek.commits` and `ProjectDetail.recentCommits`
// are lists of commits the UI shows, which reward nothing. So a `commits` field is admissible
// only when it is a list — a scalar one could only be a tally. Checking the type rather than the
// name also catches a count this test's name list would miss, such as `commitTotal: u32`.
test('no field counts commits or lines', () => {
  const banned = new Set(['commitCount', 'commitCounts', 'linesOfCode', 'lineCount', 'loc']);
  const counting = /^(commit|line)s?(total|tally|sum|num|count)?$/i;
  for (const [name, decl] of Object.entries(schema.types)) {
    if (decl.kind !== 'struct') continue;
    for (const [f, expr] of Object.entries(decl.fields)) {
      assert.ok(!banned.has(f), `${name}.${f}`);
      // §1.4's identity weight is the one admissible commit count; it does not exist yet.
      if (counting.test(f) && name !== 'IdentityRow') {
        assert.ok(parseTypeExpr(expr).array, `${name}.${f} is a scalar count of commits or lines`);
      }
    }
  }
});

// §8.4: two README fallbacks, because they are two different states. One string merges them.
test('README absence is two states, not one empty string', () => {
  assert.deepEqual(schema.types.ReadmeStateKind.variants, ['not_indexed', 'absent', 'present']);
});

// §8.5.3: the excerpt is dated by the observation that produced it — `peek_cache.computed_at`.
// NULL is not computed, and renders no age slot at all rather than a `0`.
test('a README excerpt carries when it was read', () => {
  assert.equal(schema.types.ReadmeState.fields.readAt, 'Timestamp?');
});

// §2.5: the renderer never constructs, compares, or returns a path.
test('locations.relocate is the only inbound location path, and it is privileged', () => {
  const c = schema.commands.find((x) => x.name === 'locations.relocate');
  assert.equal(c.privileged, true);
  assert.equal(c.args.pathBytes, 'Bytes');
  const trusted = schema.commands.find((x) => x.name === 'locations.setTrusted');
  assert.deepEqual(trusted.args, { locationId: 'LocationId' });
});

// §8.5.2: phase 1 compares the stored head OID of two rows. A commit count between two working
// copies needs a merge-base walk in a repository that may be offline.
test('two copies are compared by identity, never by direction', () => {
  assert.deepEqual(schema.types.HeadComparison.variants, [
    'same_commit',
    'different_commit',
    'not_compared',
  ]);
  assert.ok(!('divergedBy' in schema.types.LocationDetail.fields));
});

const REST = [
  'targets.list',
  'targets.setDefault',
  'targets.upsert',
  'targets.verify',
  'art.url',
  'art.rerender',
  'identity.list',
  'identity.confirm',
  'stats.reveal',
  'collections.list',
  'collections.upsert',
  'collections.remove',
  'session.stop',
  'session.focus',
];

test('every remaining command of §2.4, and §9 focus, is declared', () => {
  for (const n of REST) assert.ok(names.includes(n), `${n} is missing`);
});

// §2.4's table is 40 commands; §9's session.focus and §11.3a's roots.list are the 41st and 42nd
// and the only two beyond it. The literal and the transcribed lists above move together — a count
// that disagrees with them moves the failure rather than fixing it.
// [p2] §20.8's eight `accounts.*` commands are the 43rd to the 50th. Each phase-2 plan raises
// this by its OWN delta, read from the value in the file — never to a running total, which a
// lane cannot know after the merges ahead of it.
// [p2] §25.8's `remote.webUrl` is the 51st, and it is the only name §25 spends from `remote.*`.
// [p2] §25.8's `projects.readme` is the 52nd, `projects.setReadmeRemote` the 53rd and
// `projects.readmeAssets` the 54th.
// [p2] 54 off the branch base, plus §24.9's three `install.*` and §21.13's one `sync.status` =
// **58**. Two lanes each added to this number from the same base, so neither branch's figure is
// the merged one — 57 and 55 are both right about their own tree and wrong about this one.
// `locations.uninstall*` is p2-24b's and is not here.
test('the whole §2.4 table is present, plus §9 focus, roots.list, §20.8, §25.8, §24.9, §21.13 and nothing extra', () => {
  assert.equal(names.length, 58, `expected 58 commands, found ${names.length}`);
  assert.equal(new Set(names).size, names.length);
});

// §2.4 [v2.2] + criterion 7: setDefault carries the targetId it re-heads plus all three of
// §4bis.2a's scope triple. v2.2 enumerated two and made tier 1 unwritable.
test('targets.setDefault carries the full scope triple', () => {
  const c = schema.commands.find((x) => x.name === 'targets.setDefault');
  assert.deepEqual(c.args, {
    targetId: 'TargetId',
    projectId: 'ProjectId?',
    locationId: 'LocationId?',
    language: 'String?',
  });
});

// §4bis.2a: list returns the resolved row WITH the tier that resolved it — the menu labels one
// row DEFAULT and another SET, and the renderer cannot derive that distinction. Tier 5, `ask`,
// is the null resolution and is not an enum member.
test('a resolved target carries its tier, and the ask tier is a null resolution', () => {
  assert.equal(schema.types.ResolvedTarget.fields.tier, 'TargetTier');
  assert.equal(schema.types.TargetList.fields.resolved, 'ResolvedTarget?');
  assert.deepEqual(schema.types.TargetTier.variants, ['project', 'location', 'language', 'global']);
});

// §2.4: upsert takes a shell-dialog executable. §4bis.5: arguments come from the stored row as an
// argv array, never through a shell — so no executable travels back out to the renderer.
test('targets.upsert is privileged and carries exec bytes; TargetRow returns no executable', () => {
  const c = schema.commands.find((x) => x.name === 'targets.upsert');
  assert.equal(c.privileged, true);
  assert.equal(c.args.execBytes, 'Bytes');
  assert.ok(!('execBytes' in schema.types.TargetRow.fields));
  assert.equal(schema.types.TargetRow.fields.execDisplay, 'String');
});

// §7.4 + criterion 56: the absolute target offset, never an increment, one project per call.
test('art.rerender is an absolute offset for one project', () => {
  const c = schema.commands.find((x) => x.name === 'art.rerender');
  assert.deepEqual(c.args, { projectId: 'ProjectId', offset: 'u32' });
  assert.equal(schema.types.ArtRerender.fields.offset, 'u32');
  assert.equal(schema.types.ArtRerender.fields.rejected, 'bool');
});

// §7.6: a two-segment address, so a fetch of codotheca://art/<hash> fails.
test('art.url names both the hash and the rendition', () => {
  const c = schema.commands.find((x) => x.name === 'art.url');
  assert.deepEqual(c.args, { hash: 'SceneHash', rendition: 'Rendition' });
  // R47: four, and the two blueprint names carry a hyphen so each stays **one** path segment.
  // One variant cannot address two render passes — a cached raster of the card pass would be
  // served for the hero pass at exactly the moment the project changes state.
  assert.deepEqual(schema.types.Rendition.variants, [
    'card',
    'hero',
    'card-blueprint',
    'hero-blueprint',
  ]);
  for (const v of schema.types.Rendition.variants) {
    assert.doesNotMatch(v, /[/.]/, `${v} must be one path segment`);
  }
});

// §1.4: apply:false returns the delta and writes nothing — the effect is stated before the write.
test('identity.confirm can preview without writing', () => {
  const c = schema.commands.find((x) => x.name === 'identity.confirm');
  assert.deepEqual(c.args, { emails: '[String]', apply: 'bool' });
  assert.equal(schema.types.IdentityConfirm.fields.movedToReference, 'u32');
  assert.equal(schema.types.IdentityConfirm.fields.commitDaysRemoved, 'i64');
  assert.equal(schema.types.IdentityConfirm.fields.applied, 'bool');
});

// §10.4: every reveal figure carries its coverage, and says so when incomplete.
test('every reveal figure carries a basis and a nullable value', () => {
  assert.equal(schema.types.RevealFigure.fields.value, 'f64?');
  assert.equal(schema.types.RevealFigure.fields.basis, 'RevealBasis');
  assert.deepEqual(Object.keys(schema.types.RevealBasis.fields).sort(), [
    'historyComplete',
    'projectsCovered',
    'projectsTotal',
  ]);
  for (const f of ['spanDays', 'projectCount', 'languageCount', 'bestYear', 'playtimeSeconds']) {
    assert.equal(schema.types.Reveal.fields[f], 'RevealFigure', f);
  }
});

// §8.8: five named refusals, each with its own control text. A bare error would lose which.
test('collections.upsert reports which refusal fired', () => {
  assert.deepEqual(schema.types.CollectionRefusal.variants, [
    'empty_name',
    'name_taken',
    'too_long',
    'contains_collection_term',
    'limit_reached',
  ]);
  assert.equal(schema.types.CollectionUpsert.fields.refusedBecause, 'CollectionRefusal?');
});

// §2.4's topic table, transcribed. [p2] §20.8 adds `accounts`, which declares no `snapshot`:
// three events and nothing to build a frame from.
// [p2] §24.9 adds `install`: a fifth topic rather than events on `projects`, so the shelf is not
// woken at stage cadence when no install is running — the separation `scan` already has.
const TOPICS = {
  scan: ['run_started', 'repo_found', 'job_done', 'progress', 'problem', 'finished', 'cancelled'],
  install: ['started', 'stage', 'finished', 'failed', 'snapshot'],
  // [p2] §25.8 adds `readme_remote_changed` to the **existing** topic, on the `flags_changed`
  // precedent, so an optimistic flip in the renderer and the wire cannot disagree. No new topic.
  projects: [
    'upserted',
    'merged',
    'flags_changed',
    'condition_changed',
    'art_ready',
    'snapshot',
    'readme_remote_changed',
  ],
  session: ['started', 'segment_closed', 'ended'],
  core: ['error', 'degraded', 'snapshot'],
  accounts: ['connect_progress', 'connected', 'disconnected'],
  // [p2] §21.13 adds `sync`, with a `snapshot` because §8.0's notice slot and the progress line
  // both have to render correctly for a window that opened after the events fired.
  sync: ['started', 'settled', 'listing_progress', 'budget', 'notice', 'snapshot'],
};

test('every topic and event of §2.4 is declared, with a payload type', () => {
  assert.deepEqual(Object.keys(schema.topics).sort(), Object.keys(TOPICS).sort());
  for (const [t, events] of Object.entries(TOPICS)) {
    assert.deepEqual(Object.keys(schema.topics[t]).sort(), [...events].sort(), t);
    for (const e of events) assert.equal(typeof schema.topics[t][e], 'string', `${t}/${e}`);
  }
});

// [p2] §21.13: the six events are the whole of what §21 puts on the wire, and `snapshot` carries
// exactly what `sync.status` returns, so a subscriber that missed every delta still renders the
// truth.
test('the sync topic declares exactly six events and its snapshot is sync.status answer', () => {
  assert.equal(Object.keys(schema.topics.sync).length, 6);
  assert.equal(schema.topics.sync.snapshot, 'SyncStatus');
  const status = schema.commands.find((c) => c.name === 'sync.status');
  assert.equal(status.returns, 'SyncStatus');
  assert.deepEqual(status.args, {});
  assert.notEqual(status.idempotent, false, 'every sync task is a GET');
  assert.notEqual(status.privileged, true);
});

// [p2] §21.13: `SyncStatus.tasks` is the settled shape, so a queued row that has never run has a
// null outcome rather than an invented one, and `notBefore` is absent for a terminal row.
test('a sync status task is a settled row carrying its state and a nullable outcome', () => {
  assert.equal(schema.types.SyncStatus.fields.tasks, '[SyncTaskSettled]');
  const settled = schema.types.SyncTaskSettled.fields;
  assert.equal(settled.state, 'SyncTaskState');
  assert.equal(settled.outcome, 'SyncOutcomeKind?', 'a queued row has no outcome to render');
  assert.equal(settled.notBefore, 'Timestamp?');
});

// [p2] R62: §22.6 requires a suppression be reported with the project that blocked it **named**,
// and a count cannot name one. AC-P2-22-7 fails a suppression that produces no report.
test('a listing summary names the projects that suppressed, not only how many', () => {
  const summary = schema.types.SyncListingSummary.fields;
  assert.equal(summary.suppressedBy, '[ProjectId]');
  assert.equal(summary.suppressed, 'i64', 'the count stays beside the list');
  assert.equal(summary.skippedUnknownPermission, 'i64');
});

// §2.4: projects.merged did not exist in v1, so the renderer had no way to collapse two visible
// tiles into one — which happens live during the first scan.
test('projects/merged names both sides', () => {
  assert.deepEqual(schema.types.ProjectMerged.fields, { from: 'ProjectId', into: 'ProjectId' });
});

// §2.3: on overflow the queue is atomically replaced by one snapshot {epoch, throughSeq, data};
// only deltas with seq > throughSeq may follow it.
test('both snapshot events carry epoch and throughSeq', () => {
  for (const t of ['ProjectsSnapshot', 'CoreSnapshot']) {
    assert.equal(schema.types[t].fields.epoch, 'u32', t);
    assert.equal(schema.types[t].fields.throughSeq, 'i64', t);
  }
});

// §1.2: condition_signal is NULL until a scan job has produced one.
test('a condition change can carry the uncomputed state', () => {
  assert.equal(schema.types.ProjectConditionChanged.fields.conditionSignal, 'ConditionSignal?');
});

// §4.1: the job vocabulary, including J1.5, which §4.1a schedules before J2 and J3.
test('job_done names one of the eight jobs', () => {
  assert.deepEqual(schema.types.Job.variants, ['j0', 'j1', 'j1_5', 'j2', 'j3', 'j4', 'j5', 'j6']);
});

// §2.4: paths and executables enter only from a native file dialog owned by the shell, and each
// such call is a privileged capability-issuance endpoint requiring explicit user confirmation.
// Three commands qualify. Widening this set is a security decision, not a schema edit.
// [p2] §24.8 extends what `privileged` means from *carries Bytes* to *carries Bytes or mutates
// the filesystem*, and the two mutating install commands join on the second clause while
// carrying no Bytes at all. Widening this set is still a security decision, not a schema edit:
// the list is what states it, and `mutatesFilesystem` is a declared flag rather than an
// inference from a name prefix, so a command's safety class cannot depend on its spelling.
test('the privileged set is closed, and every member says which clause admits it', () => {
  const priv = schema.commands
    .filter((c) => c.privileged === true)
    .map((c) => c.name)
    .sort();
  assert.deepEqual(priv, [
    'install.cancel',
    'install.start',
    'locations.relocate',
    'roots.add',
    'targets.upsert',
  ]);
  for (const c of schema.commands.filter((x) => x.privileged === true)) {
    const bytes = Object.values(c.args).some((e) => parseTypeExpr(e).base === 'Bytes');
    assert.ok(bytes || c.mutatesFilesystem === true, c.name);
  }
});

// §2.2: non-idempotent operations are never auto-replayed — projects.launch, session mutations
// and flag changes are surfaced to the user instead. Replaying a launch opens the editor twice.
// [p2] §20.8 adds five: a replayed connect after a core restart genuinely starts a second flow,
// and a replayed disconnect deletes a keychain entry the user has since re-created.
// [p2] §25.8 adds the ninth: a consent is a decision the user made once, and replaying one
// through a core restart would re-grant it without them.
// [p2] §24.9 adds two: a replayed `install.start` after a core restart begins a second clone
// into a destination the first one is still writing, and a replayed `install.cancel` kills a
// process group that a later run may by then own.
test('the non-idempotent set is closed', () => {
  const ni = schema.commands
    .filter((c) => c.idempotent === false)
    .map((c) => c.name)
    .sort();
  assert.deepEqual(ni, [
    'accounts.connect',
    'accounts.connectPat',
    'accounts.disconnect',
    'accounts.setOrgEnabled',
    'accounts.upgradeScope',
    'install.cancel',
    'install.start',
    'projects.launch',
    'projects.setFlags',
    'projects.setReadmeRemote',
    'session.stop',
  ]);
});

// §7.4: a retried, replayed or double-delivered art.rerender writes the same integer and lands
// on the same card. It is idempotent by construction and must never join the set above.
test('art.rerender is idempotent by construction', () => {
  const c = schema.commands.find((x) => x.name === 'art.rerender');
  assert.notEqual(c.idempotent, false);
});

// §11.2a: the corrupt-index window draws three blocks and §1.12 gives them figures. v2.2 shipped
// the boolean and no numbers, so the window named what a gap contains and printed nothing.
test('the corrupt-index ledger carries a figure set for each of its three blocks', () => {
  const f = schema.types.CorruptIndexLedger.fields;
  assert.equal(f.quarantinedAt, 'Timestamp');
  assert.equal(f.gapStartedAt, 'Timestamp?');
  assert.equal(f.reDerivable, 'LedgerCounts');
  assert.equal(f.restorable, 'LedgerCounts');
  assert.equal(f.deferred, 'LedgerCounts');
});

// The boolean stays, and stays load-bearing: false means the gap block says so in words and
// prints no figure at all. §1.10 on the screen where an invented zero costs the most.
test('whether the gap could be counted is carried apart from the counts', () => {
  assert.equal(schema.types.CorruptIndexLedger.fields.gapCountsRecoverable, 'bool');
});

// §1.10 one level down: a block sourced from a struct with no column for a row kind must say
// nothing about that kind. Nullable is the only encoding left — convention 1 of this plan.
test('every ledger count is nullable, and the set is the one plan 04 computes', () => {
  const fields = schema.types.LedgerCounts.fields;
  for (const [name, expr] of Object.entries(fields)) {
    assert.equal(expr, 'i64?', `LedgerCounts.${name} must be nullable`);
  }
  assert.deepEqual(Object.keys(fields).sort(), [
    'aliases',
    'collectionMembers',
    'collections',
    'identities',
    'launchTargets',
    'merges',
    'notes',
    'projects',
    'roots',
    'sessionSegments',
    'sessions',
    'settings',
    'viewState',
    'xpEvents',
  ]);
});

// §10.5a: the NEW predicate lives once, in `isNewArrival`. ProjectRow carries its per-project
// inputs; without this field the run-wide one is missing and the chip cannot ship.
test('the core snapshot names when first run ended', () => {
  assert.equal(schema.types.CoreSnapshot.fields.firstRunCompletedAt, 'Timestamp?');
});

// §20.8's command surface. The eight names are transcribed rather than derived: a command
// silently renamed in the schema must fail here, which a `startsWith('accounts.')` filter
// could not do.
const ACCOUNTS = [
  'accounts.list',
  'accounts.orgs',
  'accounts.connect',
  'accounts.cancelConnect',
  'accounts.connectPat',
  'accounts.upgradeScope',
  'accounts.disconnect',
  'accounts.setOrgEnabled',
];

test('§20.8 declares the eight accounts commands', () => {
  for (const n of ACCOUNTS) assert.ok(names.includes(n), `${n} is missing`);
});

// §2.2's auto-replay rule: a replay of any of these five genuinely changes state. The two reads
// and `cancelConnect` are replayable, so marking them would cost a user-visible `unknown`
// outcome for nothing.
test('exactly five accounts commands refuse auto-replay', () => {
  const refusing = schema.commands
    .filter((c) => c.name.startsWith('accounts.') && c.idempotent === false)
    .map((c) => c.name)
    .sort();
  assert.deepEqual(refusing, [
    'accounts.connect',
    'accounts.connectPat',
    'accounts.disconnect',
    'accounts.setOrgEnabled',
    'accounts.upgradeScope',
  ]);
});

// `$privileged` means "may carry Bytes from a native dialog". No account command carries a path
// or an executable, and `validateSchema`'s second arm throws on a privileged command that takes
// no Bytes — so marking one would be a shape error rather than extra safety.
test('no accounts command is privileged', () => {
  const accounts = schema.commands.filter((x) => x.name.startsWith('accounts.'));
  // A loop over nothing proves nothing: without this the test reads green on a schema that
  // declares no accounts command at all.
  assert.equal(accounts.length, ACCOUNTS.length, 'the privilege check scanned the wrong set');
  for (const c of accounts) {
    assert.notEqual(c.privileged, true, `${c.name} must not be privileged`);
  }
});

// §20.4: under the public tier the org list is *unknown*, and a non-nullable array cannot say
// that — `[]` claims there are none. R41's shape and R41's answer.
test('accounts.orgs is nullable, because an unenumerable org list is unknown', () => {
  const c = schema.commands.find((x) => x.name === 'accounts.orgs');
  assert.equal(c.returns, '[AccountOrg]?');
});

// [p2] §25.8's command surface, transcribed for the same reason §20.8's is: `remote.*` is §25's
// under A11 and phase 2 spends **exactly one** name from it. A second `remote.*` command
// appearing here is a namespace growing without a ruling, and a prefix filter could not say so.
const REMOTE_COMMANDS = ['remote.webUrl'];

test('§25.8 spends exactly one name from the remote.* prefix', () => {
  const declared = names.filter((n) => n.startsWith('remote.')).sort();
  assert.deepEqual(declared, REMOTE_COMMANDS);
});

// §25.2: the answer is a URL, and NULL is the honest answer for a key this build cannot address.
// A non-nullable `String` would force the core to invent one.
test('remote.webUrl answers a nullable string and carries no Bytes', () => {
  const c = schema.commands.find((x) => x.name === 'remote.webUrl');
  assert.deepEqual(c.args, { projectId: 'ProjectId', kind: 'RemoteLinkKind' });
  assert.equal(c.returns, 'String?');
  assert.notEqual(c.privileged, true);
  assert.deepEqual(schema.types.RemoteLinkKind.variants, [
    'repository',
    'issues',
    'pulls',
    'actions',
    'releases',
  ]);
});

// [p2] §25.8's README commands. The prefix is `projects.` and not a new top-level `readme.`:
// D9 proposed `readme.assets`, and §25.8 renamed it because a one-command top-level prefix
// invites a second, forge-agnostic namespace nobody owns while these three are project-scoped.
const README_COMMANDS = ['projects.readme', 'projects.readmeAssets', 'projects.setReadmeRemote'];

test('§25.8 declares its README commands under the projects prefix and nowhere else', () => {
  const declared = names.filter((n) => /^(?:projects\.(?:readme|setReadme)|readme\.)/u.test(n));
  assert.deepEqual([...declared].sort(), [...README_COMMANDS].sort());
});

// §25.5: NULL is *never granted* and a timestamp is *granted at T*, mirroring `location.trusted_at`
// — a per-thing consent whose absence must never read as a denial the user made. `allowedAt` is
// therefore nullable on the event, and the command is non-idempotent because §2.2 never auto-
// replays a consent change.
test('projects.setReadmeRemote is a consent change, and is never auto-replayed', () => {
  const c = schema.commands.find((x) => x.name === 'projects.setReadmeRemote');
  assert.deepEqual(c.args, { projectId: 'ProjectId', allow: 'bool' });
  assert.equal(c.returns, 'Empty');
  assert.equal(c.idempotent, false);
  assert.notEqual(c.privileged, true);
  assert.equal(schema.topics.projects.readme_remote_changed, 'ReadmeRemoteChanged');
  assert.deepEqual(schema.types.ReadmeRemoteChanged.fields, {
    id: 'ProjectId',
    allowedAt: 'Timestamp?',
  });
});

// §25.5: the panel needs the document, not the paragraph. `J6_BYTE_CAP` is the cap and the
// command reads that existing constant rather than declaring a second one, so `truncated` is
// the only thing the wire adds — the panel says the document was cut rather than implying it
// ended. `state` reuses `ReadmeStateKind`: the command takes a `locationId`, so a location
// exists by construction and `not_indexed` is never borrowed for a project no pass is coming for.
test('projects.readme returns the whole document, with the state enum §8.4 already declares', () => {
  const c = schema.commands.find((x) => x.name === 'projects.readme');
  assert.deepEqual(c.args, { projectId: 'ProjectId', locationId: 'LocationId' });
  assert.equal(c.returns, 'ReadmeSource');
  assert.notEqual(c.privileged, true);
  assert.equal(schema.types.ReadmeSource.kind, 'struct');
  assert.deepEqual(schema.types.ReadmeSource.fields, {
    state: 'ReadmeStateKind',
    path: 'String?',
    text: 'String?',
    readAt: 'Timestamp?',
    truncated: 'bool',
  });
  assert.deepEqual(schema.types.ReadmeStateKind.variants, ['not_indexed', 'absent', 'present']);
});

// §25.5: one command answers both branches, because the renderer parsed one list of img[src]
// values and does not know which is which. Classification is the core's, so the rule has one
// owner — the two arrays are a hint and the core re-classifies anyway.
test('projects.readmeAssets takes both reference lists and answers one row each', () => {
  const c = schema.commands.find((x) => x.name === 'projects.readmeAssets');
  assert.deepEqual(c.args, {
    projectId: 'ProjectId',
    locationId: 'LocationId',
    local: '[String]',
    remote: '[String]',
  });
  assert.equal(c.returns, '[ReadmeAsset]');
  assert.notEqual(c.privileged, true);
  assert.deepEqual(schema.types.ReadmeAsset.fields, {
    ref: 'String',
    state: 'ReadmeAssetState',
    dataUri: 'String?',
    fetchedAt: 'Timestamp?',
  });
});

// `blocked` is the consent state and is never rendered as a failure, which is why it is a
// variant of its own rather than an absence: an asset nobody was allowed to fetch and an asset
// that could not be fetched are two different sentences.
test('ReadmeAssetState names five outcomes, and blocked is one of them', () => {
  assert.deepEqual(schema.types.ReadmeAssetState.variants, [
    'ok',
    'blocked',
    'too_large',
    'unreachable',
    'not_an_image',
  ]);
});

/** Every type name reachable from `root`, following struct fields transitively. */
function reachableTypes(root) {
  const seen = new Set();
  const queue = [root];
  while (queue.length > 0) {
    const name = queue.pop();
    if (seen.has(name)) continue;
    seen.add(name);
    const decl = schema.types[name];
    if (decl?.kind !== 'struct') continue;
    for (const expr of Object.values(decl.fields)) queue.push(parseTypeExpr(expr).base);
  }
  return seen;
}

// `Account`'s own fields are all primitives, ids and enums, so the test below cannot prove the
// walk recurses: a `reachableTypes` that queued nothing would walk `Account` and read green.
// `struct` is the only kind that has fields at all — `enum` carries string variants and `id` a
// scalar repr — so following struct fields is the whole graph, and this proves it is followed.
test('reachableTypes follows struct fields to the bottom', () => {
  const walked = reachableTypes('Problems');
  for (const name of ['Problems', 'ProblemGroup', 'ProblemItem']) {
    assert.ok(walked.has(name), `the walk stopped before ${name}`);
  }
});

// §20.8: "Account carries no token field, at any nesting depth." Structural, not a convention —
// the database stores `token_ref` and the wire carries neither it nor a secret. Walking the
// graph rather than one field list is what makes a later nested struct fail this.
test('Account carries no token field at any nesting depth', () => {
  const banned = /token|secret|credential/iu;
  const walked = reachableTypes('Account');
  let checked = 0;
  for (const name of walked) {
    const decl = schema.types[name];
    if (decl?.kind !== 'struct') continue;
    for (const field of Object.keys(decl.fields)) {
      checked += 1;
      assert.ok(!banned.test(field), `${name}.${field} puts a credential on the wire`);
    }
  }
  // Counting names would count `"String"`; counting fields counts what was actually tested.
  assert.ok(
    checked >= Object.keys(schema.types.Account.fields).length,
    `the walk tested ${checked} field(s), fewer than Account declares`,
  );
});

// [p2] §24.9's command surface, transcribed for the same reason §20.8's and §25.8's are.
// `install.*` is §24's namespace (A11); `github.*` is left unused so the provider seam is not
// pre-named after one forge. `locations.uninstall*` is p2-24b's and is declared by neither.
const INSTALL_COMMANDS = ['install.cancel', 'install.preview', 'install.start'];

test('§24.9 declares its three install commands and spends no other name on the prefix', () => {
  assert.deepEqual(names.filter((n) => n.startsWith('install.')).sort(), INSTALL_COMMANDS);
});

// §24.3a: the destination is a `RootId` and the subpath is the project's stored `seed_basename`,
// so `install.start` carries no `Bytes` at all. §24.8 is why it is privileged anyway — every
// command that mutates disk is, which is the arm Task 8 added to the validator.
test('the two mutating install commands carry no Bytes and are privileged regardless', () => {
  for (const name of ['install.start', 'install.cancel']) {
    const c = schema.commands.find((x) => x.name === name);
    assert.equal(c.privileged, true, name);
    assert.equal(c.mutatesFilesystem, true, name);
    assert.equal(c.idempotent, false, name);
    for (const [arg, expr] of Object.entries(c.args)) {
      assert.notEqual(parseTypeExpr(expr).base, 'Bytes', `${name}.${arg}`);
    }
  }
  assert.deepEqual(schema.commands.find((x) => x.name === 'install.start').args, {
    projectId: 'ProjectId',
    rootId: 'RootId',
  });
  assert.deepEqual(schema.commands.find((x) => x.name === 'install.cancel').args, {
    runId: 'InstallRunId',
  });
});

test('install.preview reads, so the page may ask before the user has committed to anything', () => {
  const c = schema.commands.find((x) => x.name === 'install.preview');
  assert.deepEqual(c.args, { projectId: 'ProjectId', rootId: 'RootId' });
  assert.equal(c.returns, 'InstallPreview');
  assert.notEqual(c.privileged, true);
  assert.notEqual(c.mutatesFilesystem, true);
  assert.notEqual(c.idempotent, false);
});

// §24.3d: a collision refuses and never auto-suffixes, and the refusal comes back as a value on
// the reply rather than as an error frame — the same shape `collections.upsert` already uses.
// Folding it into the display string is how a refusal stops being a reply, so the composed
// destination is its own type beside it rather than a sentence.
test('an install refusal is a reply, and the composed destination is its own field', () => {
  assert.equal(schema.types.InstallPreview.fields.destination, 'InstallDestination?');
  assert.equal(schema.types.InstallPreview.fields.refusedBecause, 'InstallRefusal?');
  assert.equal(schema.types.InstallStart.fields.runId, 'InstallRunId?');
  assert.equal(schema.types.InstallStart.fields.refusedBecause, 'InstallRefusal?');
  assert.deepEqual(
    schema.errors.filter((e) => /INSTALL/u.test(e)),
    [],
  );
});

// §24.3a: the renderer names a `RootId` and never composes the path. `display` is §1.10's lossy
// form and is never opened, launched or compared; `rootId` and `seedBasename` are what the core
// re-derives the real path from.
test('InstallDestination carries the display form beside the identity it was composed from', () => {
  assert.deepEqual(schema.types.InstallDestination.fields, {
    rootId: 'RootId',
    seedBasename: 'String',
    display: 'String',
  });
});

test('§24.9 declares its three install enums with exactly their variants', () => {
  assert.deepEqual(schema.types.InstallStageKind.variants, [
    'plans',
    'enumerating',
    'receiving',
    'assembling',
    'cladding',
    'settled',
  ]);
  assert.deepEqual(schema.types.InstallRefusal.variants, [
    'destination_exists',
    'already_installed',
    'unsafe_name',
    'root_unavailable',
    'no_clone_url',
    'no_install_root_chosen',
    'private_needs_upgrade',
  ]);
  assert.deepEqual(schema.types.InstallFailure.variants, [
    'network',
    'auth',
    'disk_full',
    'cancelled',
    'git_failed',
    'rename_failed',
    'filter_neutralisation_failed',
  ]);
});

// A10: `done` and `total` are per-phase and both nullable, and there is no aggregate field **by
// construction** — so no surface can render the retreating percentage §10.2 bans. A field set
// asserted exhaustively is what keeps a later author from adding one.
test('an install stage carries per-phase counts and nothing to compute a percentage from', () => {
  assert.deepEqual(Object.keys(schema.types.InstallStage.fields).sort(), [
    'bytes',
    'done',
    'runId',
    'stage',
    'total',
  ]);
  assert.equal(schema.types.InstallStage.fields.stage, 'InstallStageKind');
  for (const field of ['done', 'total', 'bytes']) {
    assert.equal(schema.types.InstallStage.fields[field], 'i64?', field);
  }
});

// R54: the topic carries a snapshot, like `scan` and `projects` and unlike `accounts` — a
// renderer that opens mid-clone has to be able to build a frame from something.
test('the install topic carries §24.9’s four events plus the snapshot', () => {
  assert.deepEqual(Object.keys(schema.topics.install).sort(), [
    'failed',
    'finished',
    'snapshot',
    'stage',
    'started',
  ]);
  assert.equal(schema.topics.install.started, 'InstallStarted');
  assert.equal(schema.topics.install.stage, 'InstallStage');
  assert.equal(schema.topics.install.finished, 'InstallFinished');
  assert.equal(schema.topics.install.failed, 'InstallFailed');
  assert.equal(schema.topics.install.snapshot, 'InstallState');
});

/**
 * R54's snapshot exists so that a tile mounting **after** the events fired shows the stage its
 * own run reached. `InstallStage` carries no `projectId` — its field set is pinned above so that
 * no aggregate can be added to it — so `runs` alone cannot say which run belongs to which
 * project, which is precisely the case the snapshot is for. The mapping rides on `started`, and
 * the join is `runId`.
 */
test('the install snapshot maps a run to its project without widening InstallStage', () => {
  assert.deepEqual(schema.types.InstallState.fields, {
    runs: '[InstallStage]',
    started: '[InstallStarted]',
  });
  assert.equal(schema.types.InstallStarted.fields.runId, 'InstallRunId');
  assert.equal(schema.types.InstallStarted.fields.projectId, 'ProjectId');
  assert.equal(schema.types.InstallStarted.fields.destinationDisplay, 'String');
  // The mapping lives there and nowhere else: a `projectId` on the stage would be one value
  // stated twice, and it would reopen the field set A10 closed.
  assert.equal(schema.types.InstallStage.fields.projectId, undefined);
});

test('the accounts topic carries exactly three events and no snapshot', () => {
  assert.deepEqual(Object.keys(schema.topics.accounts).sort(), [
    'connect_progress',
    'connected',
    'disconnected',
  ]);
  assert.equal(schema.topics.accounts.connect_progress, 'ConnectProgress');
  assert.equal(schema.topics.accounts.connected, 'Account');
  assert.equal(schema.topics.accounts.disconnected, 'AccountDisconnected');
});

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

test('the error enum is closed: §2.4 plus §1.6 PROJECT_MERGED and nothing else', () => {
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
    'STORE_OFFLINE',
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
test('the whole §2.4 table is present, plus §9 focus and roots.list, and nothing extra is', () => {
  assert.equal(names.length, 42, `expected 42 commands, found ${names.length}`);
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
  assert.deepEqual(schema.types.Rendition.variants, ['card', 'hero']);
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

// §2.4's topic table, transcribed.
const TOPICS = {
  scan: ['run_started', 'repo_found', 'job_done', 'progress', 'problem', 'finished', 'cancelled'],
  projects: ['upserted', 'merged', 'flags_changed', 'condition_changed', 'art_ready', 'snapshot'],
  session: ['started', 'segment_closed', 'ended'],
  core: ['error', 'degraded', 'snapshot'],
};

test('every topic and event of §2.4 is declared, with a payload type', () => {
  assert.deepEqual(Object.keys(schema.topics).sort(), Object.keys(TOPICS).sort());
  for (const [t, events] of Object.entries(TOPICS)) {
    assert.deepEqual(Object.keys(schema.topics[t]).sort(), [...events].sort(), t);
    for (const e of events) assert.equal(typeof schema.topics[t][e], 'string', `${t}/${e}`);
  }
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

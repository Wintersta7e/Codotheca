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

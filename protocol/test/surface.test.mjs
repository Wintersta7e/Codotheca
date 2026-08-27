import test from 'node:test';
import assert from 'node:assert/strict';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadSchema } from '../lib/schema.mjs';

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

import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
const run = (cmd, args) => spawnSync(cmd, args, { cwd: root, encoding: 'utf8' });

// The suite may run before any build step, so start from a generated tree.
run('node', ['protocol/generate.mjs']);

test('the generated files are gitignored, which is why a git status check over them is dead', () => {
  for (const p of ['app/src/generated/protocol.ts', 'core/src/protocol.rs']) {
    assert.equal(run('git', ['check-ignore', '-q', p]).status, 0, `${p} is expected to be ignored`);
  }
});

test('git status over the generated paths reports nothing even when they are modified', () => {
  const p = join(root, 'app/src/generated/protocol.ts');
  const original = readFileSync(p, 'utf8');
  try {
    writeFileSync(p, `${original}\n// hand edit\n`);
    const out = run('git', [
      'status',
      '--porcelain',
      '--',
      'app/src/generated',
      'core/src/protocol.rs',
    ]);
    assert.equal(out.stdout.trim(), '', 'the replaced CI check could never fail; this records why');
  } finally {
    writeFileSync(p, original);
  }
});

test('the lock is tracked and is not gitignored', () => {
  assert.notEqual(
    run('git', ['check-ignore', '-q', 'protocol/generated.lock']).status,
    0,
    'protocol/generated.lock must never be added to .gitignore — that is what killed the old check',
  );
  assert.equal(
    run('git', ['ls-files', '--error-unmatch', 'protocol/generated.lock']).status,
    0,
    'protocol/generated.lock must be committed',
  );
});

test('--check passes on a freshly generated tree', () => {
  assert.equal(run('node', ['protocol/generate.mjs']).status, 0);
  assert.equal(run('node', ['protocol/generate.mjs', '--check']).status, 0);
});

test('--check fails when an emitted file is hand-edited', () => {
  const p = join(root, 'core/src/protocol.rs');
  const original = readFileSync(p, 'utf8');
  try {
    writeFileSync(p, `${original}\n// hand edit\n`);
    const r = run('node', ['protocol/generate.mjs', '--check']);
    assert.equal(r.status, 1);
    assert.match(r.stderr, /core\/src\/protocol\.rs does not match the schema/);
  } finally {
    writeFileSync(p, original);
  }
});

test('the lock changes when the schema changes, which is what CI diffs', () => {
  const p = join(root, 'protocol/schema/protocol.json');
  const lockFile = join(root, 'protocol/generated.lock');
  const original = readFileSync(p, 'utf8');
  const before = readFileSync(lockFile, 'utf8');
  try {
    const s = JSON.parse(original);
    s.types['ProbeOnly'] = { kind: 'enum', variants: ['a'] };
    writeFileSync(p, `${JSON.stringify(s, null, 2)}\n`);
    assert.equal(run('node', ['protocol/generate.mjs']).status, 0);
    assert.notEqual(readFileSync(lockFile, 'utf8'), before);
  } finally {
    writeFileSync(p, original);
    run('node', ['protocol/generate.mjs']);
    assert.equal(readFileSync(lockFile, 'utf8'), before);
  }
});

test('generation is deterministic across runs', () => {
  const read = () => readFileSync(join(root, 'app/src/generated/protocol.ts'), 'utf8');
  run('node', ['protocol/generate.mjs']);
  const a = read();
  run('node', ['protocol/generate.mjs']);
  assert.equal(read(), a);
});

/**
 * The capture has one rule, and it is asserted in two halves because neither can stand alone:
 * **every capture file the CLI parses has a gate step that writes it.**
 *
 * This file is the first half — the declared list equals the writers in the tree. The second half,
 * reachability from a real gate script, runs as `captures.mjs --gate <path>` BY the gate, because
 * the gate script lives in a gitignored directory and a shipped test that read it would scan
 * nothing on a fresh clone and pass on nothing.
 */
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { CAPTURE_WRITERS, stepsMissingFrom, writersInTree } from './captures.mjs';

const root = fileURLToPath(new URL('../..', import.meta.url));
const packageJson = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));

test('the declared writers are exactly the writers the tree holds, derived and printed', () => {
  const { rows, scanned } = writersInTree(root);
  console.error(
    `captures: scanned ${String(scanned)} files, derived ${String(rows.length)} writers: ` +
      rows.map((r) => r.file).join(' '),
  );
  assert.ok(scanned > 0, 'the scan read nothing, so it proved nothing');
  const key = (r) => `${r.file} ${r.writer} ${String(r.npmScript)}`;
  assert.deepEqual(rows.map(key).sort(), CAPTURE_WRITERS.map(key).sort());
});

test('a scan that reads zero files throws rather than returning an empty set', () => {
  const empty = mkdtempSync(join(tmpdir(), 'captures-'));
  try {
    assert.throws(() => writersInTree(empty), /scanned nothing/u);
  } finally {
    rmSync(empty, { recursive: true, force: true });
  }
});

/** A gate script in the shape the real one has: one `run <name> <command>` per step. */
const gate = (steps) =>
  [
    '#!/usr/bin/env bash',
    '# a comment naming npm run check:callsites is not a step',
    'mkdir -p acceptance/results',
    'run core-test cargo test --manifest-path core/Cargo.toml --features testkit',
    'cp "$OUT/core-test.txt" acceptance/results/cargo.txt',
    'run app-test npm run test --workspace app -- --outputFile.json=../acceptance/results/vitest.json',
    '[ -f app/e2e-report.json ] && cp app/e2e-report.json acceptance/results/e2e.json',
    'run lint npm run lint',
    'run check-bundle npm run check:bundle',
    NODE_STEPS,
    'run check-forbidden npm run check:forbidden',
    'run check-problems npm run check:problems',
    ...steps,
  ].join('\n');

const NODE_STEPS = [
  'NODE_CAPTURE="--test-reporter=$ROOT/scripts/acceptance/node-reporter.mjs"',
  'run protocol-test env NODE_OPTIONS="$NODE_CAPTURE --test-reporter-destination=$ROOT/acceptance/results/node-protocol.json" npm run test --workspace protocol',
  'run harness env NODE_OPTIONS="$NODE_CAPTURE --test-reporter-destination=$ROOT/acceptance/results/node-harness.json" npm run test:harness',
].join('\n');

// The two node:test suites write their captures only when the gate asks their steps to.
test('the gate without its node:test capture flags leaves exactly those two captures missing', () => {
  const plain = gate(['run check-callsites npm run check:callsites']).replace(
    NODE_STEPS,
    'run protocol-test npm run test --workspace protocol\nrun harness npm run test:harness',
  );
  assert.deepEqual(stepsMissingFrom(plain, packageJson), [
    'node-harness.json',
    'node-protocol.json',
  ]);
});

// The measured defect this rule exists for, before and after: `script-callsites.json` was
// graded off while no gate step wrote it, so its rows were as old as the last hand run.
test('the gate without a call-site step leaves exactly that writer missing', () => {
  assert.deepEqual(stepsMissingFrom(gate([]), packageJson), ['check:callsites']);
  assert.deepEqual(
    stepsMissingFrom(gate(['run check-callsites npm run check:callsites']), packageJson),
    [],
  );
});

// Reachable, not literal. `lint:clamp` is chained into `lint:shell`, which `lint` runs, so a gate
// running `npm run lint` does write `script-motionclamp.json`. A check matching the script's name
// in the gate text would report a missing step that is not missing.
test('a writer chained into lint:shell is reached through the script graph', () => {
  assert.ok(!gate([]).includes('lint:clamp'), 'the fixture must not name the script literally');
  assert.ok(!stepsMissingFrom(gate([]), packageJson).includes('lint:clamp'));
  const noLint = gate([]).replace('run lint npm run lint', '');
  assert.ok(stepsMissingFrom(noLint, packageJson).includes('lint:clamp'));
});

test('a runner capture needs the gate to write its file, and a comment is not a step', () => {
  const noCargo = gate(['run check-callsites npm run check:callsites']).replace(
    'cp "$OUT/core-test.txt" acceptance/results/cargo.txt',
    '# cp "$OUT/core-test.txt" acceptance/results/cargo.txt',
  );
  assert.deepEqual(stepsMissingFrom(noCargo, packageJson), ['cargo.txt']);
});

// The rule catches a new writer: a script writing a capture is derived, and until it is declared
// and given a step, the tree and the declaration disagree by name. Both spellings a writer uses —
// a `join` onto the results directory, or one path ending in the file — are found.
for (const [spelling, source] of [
  ['joined', "writeFileSync(join(out, 'script-scratch.json'), '[]');\n"],
  ['one path', "writeFileSync('acceptance/results/script-scratch.json', '[]');\n"],
]) {
  test(`a new capture writer is found by the scan, by name (${spelling})`, () => {
    const tree = mkdtempSync(join(tmpdir(), 'captures-'));
    try {
      mkdirSync(join(tree, 'scripts'));
      writeFileSync(join(tree, 'scripts/check-scratch.mjs'), source);
      writeFileSync(join(tree, 'scripts/acceptance.mjs'), "if (name === 'cargo.txt') {}\n");
      writeFileSync(
        join(tree, 'package.json'),
        JSON.stringify({ scripts: { 'check:scratch': 'node scripts/check-scratch.mjs' } }),
      );
      const { rows } = writersInTree(tree);
      assert.ok(
        rows.some(
          (r) =>
            r.file === 'script-scratch.json' &&
            r.writer === 'scripts/check-scratch.mjs' &&
            r.npmScript === 'check:scratch',
        ),
        JSON.stringify(rows),
      );
    } finally {
      rmSync(tree, { recursive: true, force: true });
    }
  });
}

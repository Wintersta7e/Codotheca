/**
 * AC-P4-48-26's other half (§48.5): `check-no-updater.mjs`, unchanged, passes over the built
 * shell — no updater package in any manifest and none in the bundle.
 *
 * It runs the script exactly as `npm run lint:updater` does, against the bundle
 * `npm run build:app` wrote. With no bundle the script fails rather than passing on nothing, so
 * this test does too: it needs the build first, as the gate and CI's harness job both run it.
 */
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const script = join(dirname(fileURLToPath(import.meta.url)), 'check-no-updater.mjs');

test('ac_p4_48_26_shell', () => {
  const run = spawnSync(process.execPath, [script], { encoding: 'utf8' });
  console.log(run.stderr.trim());
  assert.equal(run.status, 0, run.stderr);
  const counted = /check-no-updater: ok \((\d+) manifests, (\d+) bundle files/u.exec(run.stderr);
  assert.ok(counted, `no ok line: ${run.stderr}`);
  assert.ok(Number(counted[1]) > 0, 'no manifest scanned');
  assert.ok(Number(counted[2]) > 0, 'no bundle file scanned');
});

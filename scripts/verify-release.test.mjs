import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { verifyRelease } from './verify-release.mjs';

const REPO = 'example-owner/example-repo';

/** A downloaded draft: two artifacts and the `SHA256SUMS` the draft job writes beside them. */
function draft(files = { 'app-0.9.0.AppImage': 'an appimage', 'app-0.9.0.exe': 'an installer' }) {
  const dir = mkdtempSync(join(tmpdir(), 'verify-release-'));
  const lines = [];
  for (const [name, body] of Object.entries(files)) {
    writeFileSync(join(dir, name), body);
    lines.push(`${createHash('sha256').update(body).digest('hex')}  ${name}`);
  }
  if (lines.length > 0) writeFileSync(join(dir, 'SHA256SUMS'), `${lines.join('\n')}\n`);
  return dir;
}

/** A `gh` that attests every file, recording each call. */
function gh(failing = new Set()) {
  const calls = [];
  const run = async (args) => {
    calls.push(args);
    const file = args[2] ?? '';
    const failed = [...failing].some((name) => file.endsWith(name));
    return failed
      ? { status: 1, stdout: '', stderr: 'no matching attestations found' }
      : { status: 0, stdout: 'Loaded 1 attestation', stderr: '' };
  };
  return { run, calls };
}

test('AC-P4-48-2 a matching draft verifies every file', async () => {
  const dir = draft();
  const { run, calls } = gh();
  const result = await verifyRelease({ dir, repo: REPO, runGh: run });
  assert.deepEqual(result.problems, []);
  assert.deepEqual(
    result.files.map((f) => [f.name, f.sha256, f.attestation]),
    [
      ['app-0.9.0.AppImage', 'ok', 'ok'],
      ['app-0.9.0.exe', 'ok', 'ok'],
    ],
  );
  assert.equal(calls.length, 2);
  for (const args of calls) {
    assert.deepEqual(args.slice(0, 2), ['attestation', 'verify']);
    assert.deepEqual(args.slice(3), ['--repo', REPO]);
  }
});

test('AC-P4-48-2 one changed byte fails, naming the file', async () => {
  const dir = draft();
  writeFileSync(join(dir, 'app-0.9.0.exe'), 'an installeR');
  const result = await verifyRelease({ dir, repo: REPO, runGh: gh().run });
  assert.equal(result.files.find((f) => f.name === 'app-0.9.0.exe')?.sha256, 'mismatch');
  assert.ok(
    result.problems.some((p) => p.includes('app-0.9.0.exe') && p.includes('SHA256SUMS')),
    JSON.stringify(result.problems),
  );
});

test('AC-P4-48-2 no SHA256SUMS fails', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'verify-release-'));
  writeFileSync(join(dir, 'app-0.9.0.exe'), 'an installer');
  const result = await verifyRelease({ dir, repo: REPO, runGh: gh().run });
  assert.ok(
    result.problems.some((p) => p.includes('no SHA256SUMS')),
    JSON.stringify(result.problems),
  );
});

test('AC-P4-48-2 zero files fails', async () => {
  const dir = draft({});
  writeFileSync(join(dir, 'SHA256SUMS'), '');
  const result = await verifyRelease({ dir, repo: REPO, runGh: gh().run });
  assert.equal(result.files.length, 0);
  assert.ok(
    result.problems.some((p) => p.includes('zero files')),
    JSON.stringify(result.problems),
  );
});

test('AC-P4-48-2 a failed attestation fails', async () => {
  const dir = draft();
  const result = await verifyRelease({
    dir,
    repo: REPO,
    runGh: gh(new Set(['app-0.9.0.AppImage'])).run,
  });
  assert.equal(result.files.find((f) => f.name === 'app-0.9.0.AppImage')?.attestation, 'failed');
  assert.ok(
    result.problems.some((p) => p.includes('app-0.9.0.AppImage') && p.includes('attestation')),
    JSON.stringify(result.problems),
  );
});

test('a downloaded file SHA256SUMS does not list fails', async () => {
  const dir = draft();
  writeFileSync(join(dir, 'extra.rpm'), 'not in the sums');
  const result = await verifyRelease({ dir, repo: REPO, runGh: gh().run });
  assert.ok(
    result.problems.some((p) => p.includes('extra.rpm')),
    JSON.stringify(result.problems),
  );
});

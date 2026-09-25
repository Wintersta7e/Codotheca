/**
 * The release workflow's shape, read as YAML (§48.2 d–f, §48.5). A static check: it proves the
 * jobs and steps are wired in the order a release needs, not that a run succeeded — the recorded
 * `workflow_dispatch` run and the tag's run are that evidence.
 */
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import yaml from 'js-yaml';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const text = readFileSync(join(root, '.github', 'workflows', 'release.yml'), 'utf8');
const workflow = /** @type {{ jobs: Record<string, any> }} */ (yaml.load(text));
const jobs = Object.entries(workflow.jobs ?? {});
const steps = jobs.flatMap(([id, job]) => (job.steps ?? []).map((step) => ({ job: id, step })));

const BUILD_STEP = /cargo build|npm run build|electron-builder/u;
const runOf = (step) => String(step.run ?? '');

process.stdout.write(
  `jobs scanned: ${String(jobs.length)}, steps scanned: ${String(steps.length)}\n`,
);

/** Every job `id` needs, directly or through another. */
function transitiveNeeds(id, seen = new Set()) {
  const needs = workflow.jobs[id]?.needs ?? [];
  for (const need of Array.isArray(needs) ? needs : [needs]) {
    if (seen.has(need)) continue;
    seen.add(need);
    transitiveNeeds(need, seen);
  }
  return seen;
}

test('release.yml was read', () => {
  assert.ok(jobs.length > 0 && steps.length > 0, 'a gate that scanned no job cannot fail');
});

test('AC-P4-48-4 release.yml names no CODOTHECA_CHANNEL and generates no notes', () => {
  assert.ok(!text.includes('CODOTHECA_CHANNEL'), 'nothing reads the channel variable');
  assert.ok(!text.includes('--generate-notes'), 'the notes are written by a person');
});

test('AC-P4-48-4 the version check precedes every build step', () => {
  const versionJobs = jobs
    .filter(([, job]) =>
      (job.steps ?? []).some((s) => runOf(s).includes('scripts/check-version.mjs')),
    )
    .map(([id]) => id);
  assert.equal(versionJobs.length, 1, 'one job runs the version check');
  const building = jobs
    .filter(([, job]) => (job.steps ?? []).some((s) => BUILD_STEP.test(runOf(s))))
    .map(([id]) => id);
  assert.ok(building.length > 0, 'the workflow builds something');
  for (const id of building) {
    assert.ok(
      transitiveNeeds(id).has(versionJobs[0]),
      `job ${id} builds without needing ${versionJobs[0]}`,
    );
  }
});

test('AC-P4-48-4 the floor step follows packaging', () => {
  const packaging = jobs.filter(([, job]) =>
    (job.steps ?? []).some((s) => runOf(s).includes('electron-builder')),
  );
  assert.ok(packaging.length > 0, 'a job packages');
  for (const [id, job] of packaging) {
    const list = job.steps ?? [];
    const pack = list.findIndex((s) => runOf(s).includes('electron-builder'));
    const floor = list.findIndex((s) => runOf(s).includes('scripts/check-glibc-floor.mjs'));
    assert.ok(floor >= 0, `job ${id} reads no glibc floor`);
    assert.ok(floor > pack, `job ${id} reads the floor before it packs`);
    assert.match(String(list[floor].if ?? ''), /Linux/u, 'the floor is read on the Linux pack');
  }
});

test('AC-P4-48-4 the draft job refuses update metadata and uses a written notes file', () => {
  const drafting = jobs.filter(([, job]) =>
    (job.steps ?? []).some((s) => runOf(s).includes('gh release create')),
  );
  assert.equal(drafting.length, 1, 'one job drafts the release');
  const list = drafting[0][1].steps;
  const create = list.findIndex((s) => runOf(s).includes('gh release create'));
  const guard = list.findIndex((s) => runOf(s).includes('scripts/check-update-metadata.mjs'));
  const notes = list.findIndex((s) => runOf(s).includes('docs/release-notes/'));
  assert.ok(guard >= 0 && guard < create, 'the metadata guard runs before the draft');
  assert.ok(notes >= 0 && notes < create, 'the notes file is checked before the draft');
  assert.match(runOf(list[create]), /--notes-file "[^"]*docs\/release-notes\/\$TAG\.md"/u);
});

test('AC-P4-48-4 a launch job runs every artifact kind from the uploaded artifact', () => {
  const launching = jobs.filter(([, job]) =>
    (job.steps ?? []).some((s) => runOf(s).includes('scripts/launch-artifact.mjs --kind')),
  );
  assert.equal(launching.length, 1, 'one job launches the artifacts');
  const [id, job] = launching[0];
  assert.ok(transitiveNeeds(id).has('build'), `${id} launches what the build job uploaded`);
  assert.ok(
    (job.steps ?? []).some(
      (s) =>
        String(s.uses ?? '').startsWith('actions/download-artifact@') &&
        String(s.with?.name ?? '').startsWith('codotheca-'),
    ),
    `${id} downloads the uploaded codotheca-* artifact rather than building one`,
  );
  const body = (job.steps ?? []).map(runOf).join('\n');
  assert.ok(!BUILD_STEP.test(body), `${id} builds nothing of its own`);
  for (const kind of ['appimage', 'deb', 'nsis', 'portable']) {
    assert.match(
      body,
      new RegExp(`launch-artifact\\.mjs --kind ${kind} `, 'u'),
      `${kind} is launched`,
    );
  }
  assert.match(body, /launch-artifact\.mjs --count-ledger/u, 'the launches are counted');
  assert.ok(
    transitiveNeeds('release').has(id),
    'the draft is made only from artifacts that launched',
  );
});

const guard = join(root, 'scripts', 'check-update-metadata.mjs');

/** @returns {{status: number, stdout: string}} */
function runGuard(dir) {
  try {
    return {
      status: 0,
      stdout: execFileSync(process.execPath, [guard, dir], { encoding: 'utf8' }),
    };
  } catch (err) {
    const e = /** @type {{status: number, stdout: string}} */ (err);
    return { status: e.status, stdout: String(e.stdout ?? '') };
  }
}

test('AC-P4-48-26 no updater: the metadata guard fails a draft carrying latest*.yml or a blockmap', () => {
  for (const name of ['latest.yml', 'Codotheca Setup 0.9.0.exe.blockmap']) {
    const dir = mkdtempSync(join(tmpdir(), 'update-metadata-'));
    writeFileSync(join(dir, name), 'x');
    writeFileSync(join(dir, 'Codotheca-0.9.0.AppImage'), 'an artifact');
    const { status, stdout } = runGuard(dir);
    assert.equal(status, 1, `${name} is update metadata`);
    assert.match(stdout, /^update metadata files: 1$/mu);
  }
  const clean = mkdtempSync(join(tmpdir(), 'update-metadata-clean-'));
  writeFileSync(join(clean, 'Codotheca-0.9.0.AppImage'), 'an artifact');
  const { status, stdout } = runGuard(clean);
  assert.equal(status, 0, stdout);
  assert.match(stdout, /^update metadata files: 0$/mu);
});

test('AC-P4-48-26 no updater: the NSIS target is built without a blockmap', () => {
  // `publish: null` stops `latest.yml`, but the NSIS target still writes `<installer>.blockmap`
  // unless its differential package is off — a real pack measured one — and the guard above
  // would then refuse every tag's draft.
  const config = /** @type {any} */ (
    yaml.load(readFileSync(join(root, 'electron-builder.yml'), 'utf8'))
  );
  assert.equal(config.publish, null, 'no publish target');
  assert.equal(config.nsis?.differentialPackage, false, 'nsis.differentialPackage is false');
});

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
import { checkVersion, declarationSites } from './check-version.mjs';

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

// core/src/protocol.rs is generated and gitignored, so a job that compiles the core before
// generating it cannot build — the first workflow_dispatch of this workflow failed on exactly that.
test('every job that compiles the core generates the protocol first', () => {
  let compiling = 0;
  for (const [id, job] of jobs) {
    const list = job.steps ?? [];
    const cargo = list.findIndex((s) => /\bcargo (build|test|clippy|check)\b/u.test(runOf(s)));
    if (cargo === -1) continue;
    compiling += 1;
    const gen = list.findIndex((s) => /\bnpm run gen\b/u.test(runOf(s)));
    assert.ok(
      gen !== -1 && gen < cargo,
      `job ${id} compiles the core before generating core/src/protocol.rs`,
    );
  }
  process.stdout.write(`jobs that compile the core: ${String(compiling)}\n`);
  assert.ok(compiling > 0, 'no job compiles the core, so nothing was checked');
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

/** A step that makes or fills the draft: it creates the release, or uploads to the one that exists. */
const DRAFTS = /gh release (create|upload)\b/u;

/** The one job that drafts, and the index of its drafting step. */
function draftingJob() {
  const drafting = jobs.filter(([, job]) => (job.steps ?? []).some((s) => DRAFTS.test(runOf(s))));
  assert.equal(drafting.length, 1, 'one job drafts the release');
  const list = drafting[0][1].steps;
  return { id: drafting[0][0], list, draft: list.findIndex((s) => DRAFTS.test(runOf(s))) };
}

test('AC-P4-48-4 the draft job refuses update metadata and uses a written notes file', () => {
  const { list, draft } = draftingJob();
  const guard = list.findIndex((s) => runOf(s).includes('scripts/check-update-metadata.mjs'));
  const notes = list.findIndex((s) => runOf(s).includes('docs/release-notes/'));
  assert.ok(guard >= 0 && guard < draft, 'the metadata guard runs before the draft');
  assert.ok(notes >= 0 && notes < draft, 'the notes file is checked before the draft');
  assert.match(runOf(list[draft]), /--notes-file "[^"]*docs\/release-notes\/\$TAG\.md"/u);
});

/** Every offset at which `needle` occurs in `hay`. */
function offsets(hay, needle) {
  const found = [];
  for (let i = hay.indexOf(needle); i !== -1; i = hay.indexOf(needle, i + 1)) found.push(i);
  return found;
}

test('AC-P4-51-3 the draft step fills a draft without --clobber, and creates one only when absent', () => {
  const { list, draft } = draftingJob();
  const body = runOf(list[draft]);
  assert.ok(
    !body.includes('--clobber'),
    '--clobber deletes the old asset before the new one lands',
  );
  const upload = body.indexOf('gh release upload "$TAG"');
  const edit = body.indexOf('gh release edit "$TAG" --notes-file "../docs/release-notes/$TAG.md"');
  const create = body.indexOf('gh release create "$TAG" --draft --verify-tag');
  const reads = offsets(body, '--json isDraft');
  process.stdout.write(`isDraft reads in the draft step: ${String(reads.length)}\n`);
  assert.ok(upload >= 0, 'an existing draft is filled by upload');
  assert.ok(edit > upload, "the draft's body is set to the notes after the upload");
  assert.ok(create >= 0, 'a tag with no release still gets a draft, as before');
  assert.equal(reads.length, 2, 'isDraft is read before the upload and again after the edit');
  assert.ok(
    reads[0] < upload && reads[1] > edit,
    'one read precedes the upload, one follows the edit',
  );
  assert.ok(
    reads[0] > body.indexOf('> SHA256SUMS'),
    'the release is read once the checksums exist',
  );
});

test('AC-P4-51-3 the draft step tells absent from unreadable, and refuses a published release', () => {
  const { list, draft } = draftingJob();
  const body = runOf(list[draft]);
  const upload = body.indexOf('gh release upload');
  // gh's own words for a tag with no release; any other failure must not reach the create.
  assert.match(body, /grep -qx 'release not found'/u, 'absent is told by gh saying so');
  assert.match(body, /could not read the release/u, 'another gh failure stops the step');
  const published = body.search(/is already published/u);
  assert.ok(published >= 0, 'a published release has its own failing branch');
  assert.ok(published < upload, 'the published branch is decided before any upload');
  assert.match(body, /was published while/u, 'a draft published mid-run fails loudly');
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

// §51 — release-please keeps the release PR open, and its merge tags and drafts. These read the
// workflow and the config the action reads; whether release-please writes what they say is the
// recorded dry run's evidence, not this file's.

/** A repository file's text, or `null` when it does not exist — so one absent file fails its own tests. */
function readOptional(rel) {
  try {
    return readFileSync(join(root, rel), 'utf8');
  } catch {
    return null;
  }
}

function loadRequired(rel, parse) {
  const raw = readOptional(rel);
  assert.ok(raw !== null, `${rel} is missing`);
  return parse(raw);
}

const releasePleaseYml = () =>
  /** @type {any} */ (loadRequired('.github/workflows/release-please.yml', yaml.load));
const releasePleaseConfig = () =>
  /** @type {any} */ (loadRequired('release-please-config.json', JSON.parse));
const RELEASE_PLEASE_ACTION = 'googleapis/release-please-action@';

/** The action's step and every step of the job that holds it. */
function releasePleaseJob() {
  const wf = releasePleaseYml();
  const found = Object.entries(wf.jobs ?? {}).filter(([, job]) =>
    (job.steps ?? []).some((s) => String(s.uses ?? '').startsWith(RELEASE_PLEASE_ACTION)),
  );
  assert.equal(found.length, 1, 'one job runs the release-please action');
  const [id, job] = found[0];
  const action = job.steps.find((s) => String(s.uses ?? '').startsWith(RELEASE_PLEASE_ACTION));
  return { wf, id, job, action };
}

test('AC-P4-51-1 release-please.yml grants nothing at the top and four writes to its one job', () => {
  const { wf, id, job } = releasePleaseJob();
  assert.deepEqual(wf.permissions, {}, 'top-level permissions are {}');
  assert.deepEqual(
    Object.entries(job.permissions ?? {}).sort(),
    [
      ['actions', 'write'],
      ['contents', 'write'],
      ['issues', 'write'],
      ['pull-requests', 'write'],
    ],
    `job ${id} holds exactly the four writes §51.2 lists`,
  );
  assert.deepEqual(wf.on?.push?.branches, ['main'], 'it runs on a push to main');
});

test('AC-P4-51-1 the release-please action is pinned by a full commit SHA and reads the root files', () => {
  const { action } = releasePleaseJob();
  assert.match(String(action.uses), /@[0-9a-f]{40}$/u, 'pinned by a 40-hex commit SHA');
  assert.equal(action.id, 'release', 'the dispatch conditions read steps.release');
  assert.equal(action.with?.['config-file'], 'release-please-config.json');
  assert.equal(action.with?.['manifest-file'], '.release-please-manifest.json');
});

test('AC-P4-51-1 release.yml is dispatched only on release_created, ci.yml only on a PR', () => {
  const { job } = releasePleaseJob();
  const list = job.steps ?? [];
  const dispatching = (wf) => list.filter((s) => runOf(s).includes(`gh workflow run ${wf}`));
  const release = dispatching('release.yml');
  const ci = dispatching('ci.yml');
  process.stdout.write(
    `dispatch steps: release.yml ${String(release.length)}, ci.yml ${String(ci.length)}\n`,
  );
  assert.equal(release.length, 1, 'one step dispatches release.yml');
  assert.equal(ci.length, 1, 'one step dispatches ci.yml');
  assert.equal(
    String(release[0].if ?? '').replace(/\s+/gu, ''),
    "steps.release.outputs.release_created=='true'",
    'release.yml is dispatched only when a release was created',
  );
  assert.equal(
    String(ci[0].if ?? '').trim(),
    'steps.release.outputs.pr',
    'ci.yml is dispatched only when the action reports a PR',
  );
  for (const step of [...release, ...ci]) {
    assert.ok(!runOf(step).includes('${{'), 'values reach the shell through env, never inline');
  }
});

test('AC-P4-51-1 the config drafts the release and creates its tag', () => {
  const pkg = releasePleaseConfig().packages?.['.'];
  assert.ok(pkg, 'the config has package "."');
  assert.equal(pkg.draft, true, 'draft is true: the merge makes a private draft, never a release');
  assert.equal(
    pkg['force-tag-creation'],
    true,
    'force-tag-creation is true: a draft without its tag hides the previous release',
  );
});

test('AC-P4-51-1 no workflow or config carries a Release-As trailer or a release-as input', () => {
  const texts = {
    '.github/workflows/release-please.yml': readOptional('.github/workflows/release-please.yml'),
    '.github/workflows/release.yml': text,
    '.github/workflows/ci.yml': readOptional('.github/workflows/ci.yml'),
    'release-please-config.json': readOptional('release-please-config.json'),
  };
  for (const [rel, body] of Object.entries(texts)) {
    assert.ok(body !== null, `${rel} is missing`);
    assert.ok(!body.includes('Release-As'), `${rel} names the Release-As trailer`);
  }
  const { action } = releasePleaseJob();
  assert.ok(!('release-as' in (action.with ?? {})), 'the action is given no release-as input');
});

/**
 * Where release-please writes a site, spelled as the extra-files entry that would name it. The
 * `node` release type writes the root `package.json` and the lockfile's two root entries itself.
 */
const NODE_WRITES = new Set([
  'package.json#version',
  'package-lock.json#version',
  'package-lock.json#packages[""].version',
]);

/** `<file>#<field>` as check-version.mjs spells a site → the `{ type, path, jsonpath }` naming it. */
function extraFileFor(site) {
  const [path, field] = site.split('#');
  const type = /\.json$/u.test(path) ? 'json' : 'toml';
  const keyed = /^packages\["([^"]+)"\]\.version$/u.exec(field);
  const named = /^package\["([^"]+)"\]\.version$/u.exec(field);
  if (keyed) return { type, path, jsonpath: `$.packages.${keyed[1]}.version` };
  // release-please's TOML updater parses each value into a node holding it as `.value`, so a
  // filter on a plain `@.name` matches no entry and the lock is silently left stale.
  if (named) return { type, path, jsonpath: `$.package[?(@.name.value=='${named[1]}')].version` };
  return { type, path, jsonpath: `$.${field}` };
}

test('AC-P4-51-1 every version site check-version.mjs derives is written by the release PR', () => {
  const pkg = releasePleaseConfig().packages?.['.'];
  assert.equal(pkg?.['release-type'], 'node', 'package "." is a node release');
  const extra = (pkg['extra-files'] ?? []).filter((e) => typeof e === 'object');
  const sites = declarationSites(root).map((s) => s.site);
  const uncovered = [];
  for (const site of sites) {
    if (NODE_WRITES.has(site)) continue;
    const want = extraFileFor(site);
    const hit = extra.some(
      (e) => e.type === want.type && e.path === want.path && e.jsonpath === want.jsonpath,
    );
    if (!hit) uncovered.push(`${site} (wants ${JSON.stringify(want)})`);
  }
  process.stdout.write(`sites covered: ${String(sites.length - uncovered.length)}\n`);
  assert.ok(sites.length > 0, 'no version site was derived, so nothing was checked');
  assert.deepEqual(uncovered, [], 'every site is written by the node type or an extra-files entry');
});

test('AC-P4-51-1 the manifest holds the version every site holds, and the README line is marked', () => {
  const manifest = loadRequired('.release-please-manifest.json', JSON.parse);
  const { version, problems } = checkVersion({ root, tag: null });
  assert.deepEqual(problems, []);
  assert.equal(manifest['.'], version, 'the manifest states the version the sites agree on');
  const extra = releasePleaseConfig().packages?.['.']?.['extra-files'] ?? [];
  assert.ok(
    extra.some((e) => e === 'README.md' || (e?.type === 'generic' && e?.path === 'README.md')),
    'README.md is a generic extra-file',
  );
  const marked = String(readOptional('README.md'))
    .split('\n')
    .filter((l) => l.includes('x-release-please-version'));
  assert.equal(marked.length, 1, 'one README line carries the version marker');
  assert.ok(marked[0].includes(`v${version}`), `the marked README line states v${version}`);
});

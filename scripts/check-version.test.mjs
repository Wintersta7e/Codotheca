import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { checkVersion, declarationSites } from './check-version.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const checker = join(root, 'scripts', 'check-version.mjs');

const git = (cwd, ...args) => execFileSync('git', args, { cwd, encoding: 'utf8', stdio: 'pipe' });

/** Every manifest the repository tracks, by the same pathspecs the checker is told nothing about. */
function trackedManifests() {
  return git(root, 'ls-files', '-z', '--', '*package.json', 'package-lock.json', '*Cargo.*')
    .split('\0')
    .filter((p) => /(^|\/)(package\.json|package-lock\.json|Cargo\.toml|Cargo\.lock)$/u.test(p));
}

/**
 * The expected site count, each term read from the files themselves — never a literal. This is the
 * other side of the mirror: it does not call the derivation it is checking.
 */
function expectedSiteCount() {
  const workspaces = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).workspaces;
  const cargoPackages = git(root, 'ls-files', '-z', '--', '*Cargo.toml')
    .split('\0')
    .filter(Boolean)
    .filter((p) => /^\[package\]\s*$/mu.test(readFileSync(join(root, p), 'utf8'))).length;
  return 1 + workspaces.length + (1 + 1 + workspaces.length) + 2 * cargoPackages;
}

/**
 * A fresh git repository holding a copy of this repository's manifests, with every declaration set
 * to `version`. The rewrite is written here by hand, site by site, so a site the derivation forgets
 * still carries the fixture's value and shows up as a count the test can see.
 */
function fixtureRoot(version) {
  const dir = mkdtempSync(join(tmpdir(), 'check-version-'));
  git(dir, 'init', '-q');
  const files = trackedManifests();
  for (const file of files) {
    mkdirSync(dirname(join(dir, file)), { recursive: true });
    copyFileSync(join(root, file), join(dir, file));
  }
  const edit = (file, change) =>
    writeFileSync(join(dir, file), change(readFileSync(join(dir, file), 'utf8')));
  const pkg = JSON.parse(readFileSync(join(dir, 'package.json'), 'utf8'));
  for (const file of ['package.json', ...pkg.workspaces.map((ws) => `${ws}/package.json`)]) {
    edit(file, (text) => `${JSON.stringify({ ...JSON.parse(text), version }, null, 2)}\n`);
  }
  edit('package-lock.json', (text) => {
    const lock = JSON.parse(text);
    lock.version = version;
    lock.packages[''].version = version;
    for (const ws of pkg.workspaces) lock.packages[ws].version = version;
    return `${JSON.stringify(lock, null, 2)}\n`;
  });
  for (const toml of files.filter((f) => f.endsWith('Cargo.toml'))) {
    const text = readFileSync(join(dir, toml), 'utf8');
    const name = /^\[package\][^[]*?^name = "([^"]+)"/mu.exec(text)?.[1];
    if (name === undefined) continue;
    edit(toml, (t) => t.replace(/^(\[package\][^[]*?^version = )"[^"]*"/mu, `$1"${version}"`));
    const lock = join(dirname(toml), 'Cargo.lock');
    edit(lock, (t) =>
      t.replace(new RegExp(`^(name = "${name}"\\nversion = )"[^"]*"`, 'mu'), `$1"${version}"`),
    );
  }
  git(dir, 'add', '--', ...files);
  return dir;
}

/** @returns {{status: number, stdout: string, stderr: string}} */
function runCli(args, env) {
  try {
    const stdout = execFileSync(process.execPath, [checker, ...args], {
      encoding: 'utf8',
      stdio: 'pipe',
      env: { ...process.env, GITHUB_REF_TYPE: '', GITHUB_REF_NAME: '', ...env },
    });
    return { status: 0, stdout, stderr: '' };
  } catch (err) {
    const e = /** @type {{status: number, stdout: string, stderr: string}} */ (err);
    return { status: e.status, stdout: String(e.stdout ?? ''), stderr: String(e.stderr ?? '') };
  }
}

test('AC-P4-48-1 a v0.9.0 tag over a tree declaring 1.0.0 fails and names every site', () => {
  const dir = fixtureRoot('1.0.0');
  const result = checkVersion({ root: dir, tag: 'v0.9.0' });
  assert.equal(result.sites.length, expectedSiteCount(), 'every declaration is read');
  for (const { site, value } of result.sites) {
    assert.equal(value, '1.0.0', `${site} carries the fixture's value`);
    assert.ok(
      result.problems.some((p) => p.includes(site)),
      `${site} is named: ${result.problems.join(' | ')}`,
    );
  }
  assert.equal(result.version, '1.0.0', 'the declarations agree with one another');
});

test('AC-P4-48-1 one disagreeing lock site fails and is named', () => {
  const dir = fixtureRoot('0.9.0');
  const lockPath = join(dir, 'package-lock.json');
  const lock = JSON.parse(readFileSync(lockPath, 'utf8'));
  lock.packages.protocol.version = '0.9.1';
  writeFileSync(lockPath, JSON.stringify(lock, null, 2));
  const result = checkVersion({ root: dir, tag: null });
  assert.equal(result.version, null, 'a disagreement leaves no version to compare');
  assert.equal(result.problems.length, 1, result.problems.join(' | '));
  assert.match(result.problems[0], /package-lock\.json#packages\["protocol"\]\.version = 0\.9\.1/u);
});

test('AC-P4-48-1 zero sites read fails', () => {
  const dir = mkdtempSync(join(tmpdir(), 'check-version-empty-'));
  git(dir, 'init', '-q');
  const result = checkVersion({ root: dir, tag: null });
  assert.equal(result.sites.length, 0);
  assert.ok(
    result.problems.some((p) => /no version declaration was read/u.test(p)),
    result.problems.join(' | '),
  );
});

test('AC-P4-48-1 a prerelease-shaped tag fails', () => {
  const dir = fixtureRoot('0.9.0');
  const result = checkVersion({ root: dir, tag: 'v0.9.0-beta.1' });
  assert.equal(result.version, '0.9.0');
  assert.ok(
    result.problems.some((p) => /v0\.9\.0-beta\.1.*v<major>\.<minor>\.<patch>/u.test(p)),
    result.problems.join(' | '),
  );
});

test('AC-P4-48-1 no tag passes and says it compared none', () => {
  const dir = fixtureRoot('0.9.0');
  assert.deepEqual(checkVersion({ root: dir, tag: null }).problems, []);
  // The CLI over the real tree, as the `workflow_dispatch` run sees it: a branch ref, no tag.
  const { status, stdout, stderr } = runCli([], {
    GITHUB_REF_TYPE: 'branch',
    GITHUB_REF_NAME: 'main',
  });
  assert.equal(status, 0, stderr);
  assert.match(stdout, /^no tag compared \(branch main\)$/mu);
});

test("AC-P4-48-1 the tracked tree's sites are derived, not listed", () => {
  const sites = declarationSites(root);
  const expected = expectedSiteCount();
  process.stdout.write(
    `version sites derived: ${String(sites.length)}, expected ${String(expected)}\n`,
  );
  assert.ok(expected > 0, 'the expectation read something');
  assert.equal(sites.length, expected, sites.map((s) => s.site).join('\n'));
  assert.equal(new Set(sites.map((s) => s.site)).size, sites.length, 'no site is read twice');
});

test('the CLI exits 2 on an argument it does not know', () => {
  const { status, stderr } = runCli(['--tga', 'v0.9.0'], {});
  assert.equal(status, 2, stderr);
});

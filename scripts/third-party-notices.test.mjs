import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import {
  bundledPackageNames,
  npmPackage,
  reachableCrates,
  renderNotices,
} from './third-party-notices.mjs';

function crateDir(name, licence) {
  const dir = mkdtempSync(join(tmpdir(), `crate-${name}-`));
  writeFileSync(join(dir, 'Cargo.toml'), '');
  if (licence !== undefined) writeFileSync(join(dir, 'LICENSE-MIT'), licence);
  return join(dir, 'Cargo.toml');
}

const normal = [{ kind: null, target: null }];
const dev = [{ kind: 'dev', target: null }];
const build = [{ kind: 'build', target: null }];

/** One `cargo metadata` document: root → a (normal) → b (normal); root → d (dev); root → s (build). */
function metadata(extraDep) {
  const pkg = (name, license) => ({
    id: `${name}-id`,
    name,
    version: '1.0.0',
    license,
    license_file: null,
    manifest_path: crateDir(name, `${name} licence text`),
  });
  const packages = [
    pkg('root', 'GPL-3.0'),
    pkg('a', 'MIT'),
    pkg('b', 'Apache-2.0'),
    pkg('d', 'MIT'),
    pkg('s', 'MIT'),
  ];
  if (extraDep) packages.push(pkg(extraDep, 'MIT'));
  const rootDeps = [
    { pkg: 'a-id', dep_kinds: normal },
    { pkg: 'd-id', dep_kinds: dev },
    { pkg: 's-id', dep_kinds: build },
  ];
  if (extraDep) rootDeps.push({ pkg: `${extraDep}-id`, dep_kinds: normal });
  return {
    packages,
    resolve: {
      root: 'root-id',
      nodes: [
        { id: 'root-id', deps: rootDeps },
        { id: 'a-id', deps: [{ pkg: 'b-id', dep_kinds: normal }] },
        { id: 'b-id', deps: [] },
        { id: 'd-id', deps: [{ pkg: 'a-id', dep_kinds: normal }] },
        { id: 's-id', deps: [] },
      ],
    },
  };
}

test('AC-P4-50-3 the notices name the crates the core links and skip dev and build dependencies', () => {
  const crates = reachableCrates([metadata(), metadata('winonly')]);
  assert.deepEqual(
    crates.map((crate) => `${crate.name} ${crate.license}`),
    ['a MIT', 'b Apache-2.0', 'winonly MIT'],
  );
  assert.equal(readFileSync(crates[0].texts[0], 'utf8'), 'a licence text');
});

test('AC-P4-50-3 the notices name every bundled npm package and electron', () => {
  const names = bundledPackageNames({
    chunks: [{ packages: ['markdown-it', 'mdurl'] }, { packages: ['mdurl'] }],
  });
  assert.deepEqual(names, ['electron', 'markdown-it', 'mdurl']);

  const modules = mkdtempSync(join(tmpdir(), 'modules-'));
  mkdirSync(join(modules, 'mdurl'));
  writeFileSync(join(modules, 'mdurl', 'package.json'), '{"version":"2.1.0","license":"MIT"}');
  writeFileSync(join(modules, 'mdurl', 'LICENSE'), 'mdurl licence');
  const entry = npmPackage('mdurl', [join(modules, 'missing'), modules]);
  assert.equal(
    `${entry.name} ${entry.version} ${entry.license} ${String(entry.texts.length)}`,
    'mdurl 2.1.0 MIT 1',
  );
  assert.throws(() => npmPackage('absent', [modules]), /absent is installed in none/u);
});

test('AC-P4-50-3 the notices generator fails at zero entries', () => {
  const crates = reachableCrates([metadata()]);
  const packages = [{ name: 'electron', version: '1.0.0', license: 'MIT', texts: [] }];
  assert.throws(() => renderNotices([], packages), /zero crates/u);
  assert.throws(() => renderNotices(crates, []), /zero npm packages/u);

  const text = renderNotices(crates, packages);
  assert.match(text, /^a 1\.0\.0 — MIT — text \[1\]$/mu);
  assert.match(text, /^electron 1\.0\.0 — MIT — no licence text shipped$/mu);
  assert.match(text, /^a licence text$/mu);
});

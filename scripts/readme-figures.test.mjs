/**
 * §48.1: a figure a published document quotes is the tagged tree's generator output.
 *
 * The register's figures are `renderRegistryLine`'s — the line `npm run acceptance` prints — and
 * the version is the root `package.json`'s, which `check-version.mjs` holds every other
 * declaration to. Nothing here is a literal, so a README that drifts from either fails.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { loadRegistry } from './acceptance/registry.mjs';
import { renderRegistryLine } from './acceptance/report.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const readme = readFileSync(join(root, 'README.md'), 'utf8');

/** Every number the text attaches to *criteria* or *checks*, with the noun it counts. */
function registerFigures(text) {
  return [...text.matchAll(/(\d[\d,]*)\s+(?:written\s+)?(criteria|checks)\b/gi)].map((m) => ({
    value: Number(m[1].replace(/,/g, '')),
    noun: m[2].toLowerCase(),
    quoted: m[0],
  }));
}

test("AC-P4-48-11 the register figures the README quotes are the register's", () => {
  const line = renderRegistryLine(loadRegistry(join(root, 'acceptance/criteria.json')));
  const derived = /^(\d+) criteria \/ (\d+) checks/.exec(line);
  assert.ok(derived, `renderRegistryLine gave no figures: ${line}`);
  const expected = { criteria: Number(derived[1]), checks: Number(derived[2]) };
  const quoted = registerFigures(readme);
  console.log(
    `register: ${String(expected.criteria)} criteria / ${String(expected.checks)} checks; README quotes: ${quoted.map((q) => q.quoted).join('; ')}`,
  );
  assert.ok(quoted.length > 0, 'the README quotes no register figure, so nothing was compared');
  for (const q of quoted) {
    assert.equal(
      q.value,
      expected[q.noun],
      `README quotes "${q.quoted}"; the register holds ${String(expected[q.noun])}`,
    );
  }
});

test("README row 8: the version the README states is package.json's", () => {
  const { version } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
  const [major, minor, patch] = version.split('.');
  const status = /^## Status\n([\s\S]*?)(?=^## )/m.exec(readme);
  assert.ok(status, 'the README has no Status section');
  const stated = [...status[1].matchAll(/\bv(\d+)\.(\d+)\.(\d+|x)\b/g)];
  console.log(`package.json: ${version}; Status states: ${stated.map((m) => m[0]).join(', ')}`);
  assert.ok(stated.length > 0, 'the Status section states no version');
  for (const m of stated) {
    assert.ok(
      m[1] === major && m[2] === minor && (m[3] === 'x' || m[3] === patch),
      `the Status section states ${m[0]}; package.json declares ${version}`,
    );
  }
});

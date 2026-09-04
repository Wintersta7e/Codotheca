#!/usr/bin/env node
// Criterion 61's static half. The accelerator for the Windows system window menu is never a
// default and never a literal in the shell bundle (§8.6, §11.7); it reaches globalShortcut only
// as a chord the user recorded at runtime. The renderer bundle is deliberately out of scope —
// §8.0a's chip reads the chord as a label and is allowed to.
//
// This file never spells the chord either, or it would fail its own check the moment anyone
// pointed it at the scripts directory.
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const bundle = join(root, 'app/out/main/index.js');
const shown = relative(root, bundle);
// Assembled rather than written, or this file would fail its own check. Compared with
// `includes` and not with `new RegExp(CHORD, 'i')`: in a pattern the `+` is a quantifier, so
// that regex matches `AltSpace` and never the literal it exists to catch. Measured — a
// `console.error("Alt" + "+Space")` in the shell entry passed the regex form of this gate.
const CHORD = ['A', 'l', 't', '+', 'S', 'p', 'a', 'c', 'e'].join('').toLowerCase();

let source;
try {
  source = readFileSync(bundle, 'utf8');
} catch {
  console.error(`check-shell-accelerators: ${shown} is missing — run \`npm run build:app\` first`);
  process.exit(1);
}

// A gate whose passing run reads nothing is a failing gate that looks green.
if (source.length === 0) {
  console.error(`check-shell-accelerators: ${shown} is empty`);
  process.exit(1);
}

const failures = [];

// The bundle alone is not enough: until something in the shell entry imports the shortcut
// module, tree-shaking leaves it out and this gate passes over a bundle carrying none of the
// code it polices. The shell's own source is always there to read, so it is read too, and both
// counts are printed.
// `withFileTypes`, never a second `statSync`: a parallel test's probe can be gone between the
// readdir and the stat, and an ENOENT there takes the gate down instead of reporting. See
// `scripts/lib/read-scanned.mjs`, which guards the same race one step later at the read.
function shellSources(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...shellSources(full));
    else if (entry.name.endsWith('.ts') && !entry.name.endsWith('.test.ts')) out.push(full);
  }
  return out;
}

const sources = shellSources(join(root, 'app/src/main'));
if (sources.length === 0) {
  console.error('check-shell-accelerators: found no shell source under app/src/main');
  process.exit(1);
}
for (const file of sources) {
  if (readFileSync(file, 'utf8').toLowerCase().includes(CHORD)) {
    failures.push(`${relative(root, file)} spells the system window menu accelerator`);
  }
}

if (source.toLowerCase().includes(CHORD)) {
  failures.push(`${shown} contains the system window menu accelerator as a literal`);
}

// A registration whose accelerator is written inline is a default binding by another name.
const literalRegistration = /globalShortcut\s*\.\s*register\s*\(\s*['"`]/;
if (literalRegistration.test(source)) {
  failures.push(`${shown} calls globalShortcut.register with a string literal`);
}

if (failures.length > 0) {
  for (const line of failures) console.error(`check-shell-accelerators: ${line}`);
  process.exit(1);
}

console.error(
  `check-shell-accelerators: ok — ${String(sources.length)} shell source(s) and ` +
    `${String(source.length)} bytes of ${shown}, no accelerator literal`,
);

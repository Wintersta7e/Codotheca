#!/usr/bin/env node
// Criterion 53's half. `clip-path` clips an `outline` away entirely, so a rule that removes the
// outline without drawing a replacement in the same rule leaves a keyboard user with nothing.
// The replacement is an inset ring in the element's own box-shadow — never an overlay layer,
// one of which silently failed to mount in design review.
import { readdirSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readScannedFile } from './lib/read-scanned.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const rendererDir = join(root, 'app/src/renderer');
const failures = [];

function files(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) out.push(...files(full));
    else if (entry.endsWith('.css') || entry.endsWith('.tsx')) out.push(full);
  }
  return out;
}

const hasInsetRing = (block) => /box-?[sS]hadow[^;]*inset[^;]*/.test(block);

const walked = files(rendererDir);
const scanned = [];
for (const file of walked) {
  const text = readScannedFile(file);
  // Not counted when it has vanished, so the empty-scan guard below still means what it says.
  if (text === null) continue;
  scanned.push(file);
  for (const block of text.split(/\}/)) {
    if (!/outline\s*:\s*(none|0)\b/.test(block) && !/outline\s*:\s*['"]?none/.test(block)) continue;
    if (!hasInsetRing(block)) {
      failures.push(
        `${relative(root, file)}: outline removed with no inset box-shadow ring in the same rule`,
      );
    }
  }
}

// A gate whose passing run reads nothing is a failing gate that looks green.
if (scanned.length === 0) {
  console.error(`check-focus-ring: found no source file under ${relative(root, rendererDir)}`);
  process.exit(1);
}

const summary = `${String(scanned.length)} file(s)`;

if (failures.length > 0) {
  for (const line of failures) console.error(`check-focus-ring: ${line}`);
  console.error(`check-focus-ring: ${String(failures.length)} problem(s) over ${summary}`);
  process.exit(1);
}
console.error(`check-focus-ring: ok — ${summary}`);

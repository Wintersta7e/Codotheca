#!/usr/bin/env node
// Criterion 53's half. `clip-path` clips an `outline` away entirely, so a rule that removes the
// outline without drawing a replacement in the same rule leaves a keyboard user with nothing.
// The replacement is an inset ring in the element's own box-shadow — never an overlay layer,
// one of which silently failed to mount in design review.
import { readdirSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readScannedFile } from './lib/read-scanned.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const rendererDir = join(root, 'app/src/renderer');
const failures = [];

// `withFileTypes`, never a second `statSync`: a parallel test's probe can be gone between the
// readdir and the stat, and an ENOENT there takes the gate down instead of reporting. See
// `scripts/lib/read-scanned.mjs`, which guards the same race one step later at the read.
function files(dir) {
  const out = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...files(full));
    else if (entry.name.endsWith('.css') || entry.name.endsWith('.tsx')) out.push(full);
  }
  return out;
}

const hasInsetRing = (block) => /box-?[sS]hadow[^;]*inset[^;]*/.test(block);

/** A rule's selectors, comments stripped, and its body; `null` for text holding no rule. */
function ruleOf(block) {
  const at = block.indexOf('{');
  if (at < 0) return null;
  const selectors = block
    .slice(0, at)
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .split(',')
    .map((selector) => selector.trim())
    .filter((selector) => selector.length > 0);
  return { selectors, body: block.slice(at + 1) };
}

// Criterion 53 asks for exactly one indicator, so a control inside a composite field may hand
// its ring to the field. Every selector of the removal names that field — `<field>:focus-within
// <control>` — and the field's own `<field>:focus-within` rule in the same file draws a border or
// an inset ring. Deferring to a field that draws nothing is still a removal with no replacement.
const FOCUS_WITHIN_OWNER = /^(.+?:focus-within)\s+\S/;
const drawsIndicator = (body) =>
  hasInsetRing(body) || /border(-color)?\s*:\s*(?!0\b|none\b|transparent\b)[^;\s]/.test(body);

function deferredToField(block, rules) {
  const own = ruleOf(block);
  if (own === null || own.selectors.length === 0) return false;
  return own.selectors.every((selector) => {
    const owner = FOCUS_WITHIN_OWNER.exec(selector)?.[1];
    return (
      owner !== undefined &&
      rules.some((rule) => rule.selectors.includes(owner) && drawsIndicator(rule.body))
    );
  });
}

const walked = files(rendererDir);
const scanned = [];
for (const file of walked) {
  const text = readScannedFile(file);
  // Not counted when it has vanished, so the empty-scan guard below still means what it says.
  if (text === null) continue;
  scanned.push(file);
  const blocks = text.split(/\}/);
  const rules = blocks.map(ruleOf).filter((rule) => rule !== null);
  for (const block of blocks) {
    if (!/outline\s*:\s*(none|0)\b/.test(block) && !/outline\s*:\s*['"]?none/.test(block)) continue;
    if (!hasInsetRing(block) && !deferredToField(block, rules)) {
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

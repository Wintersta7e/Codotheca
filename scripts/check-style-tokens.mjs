#!/usr/bin/env node
// Criterion 46, mechanically. The renderer's own stylesheet declares no colour outside
// tokens.css and no duration or timing function the spec does not carry.
//
// It is a grep over CSS rather than a DOM audit because that is what the criterion specifies,
// and because a DOM audit cannot see a rule no fixture happens to trigger.
//
// Vendored @fontsource faces are third-party CSS in the built bundle and are not this
// stylesheet; plan 20 owns the built-bundle form with that carve-out named.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const rendererDir = join(root, 'app/src/renderer');
const vocabulary = JSON.parse(
  readFileSync(join(rendererDir, 'styles/css-vocabulary.json'), 'utf8'),
);

const failures = [];
const fail = (file, message) => failures.push(`${relative(root, file)}: ${message}`);

function cssFiles(dir) {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) out.push(...cssFiles(full));
    else if (entry.endsWith('.css')) out.push(full);
  }
  return out;
}

const tokensFile = join(rendererDir, 'styles/tokens.css');
const tokenNames = new Set(
  [...readFileSync(tokensFile, 'utf8').matchAll(/^\s*--([a-z0-9-]+)\s*:/gm)].map((m) => m[1]),
);
if (tokenNames.size === 0) fail(tokensFile, 'declares no custom property');

// §5.4a owns the condition-dot fills outright and satisfies criterion 46 there. They reach the
// DOM as inline style; if one ever appears in the stylesheet it is still legal.
const CONDITION_DOT_FILLS = new Set([
  '#4a9dff',
  '#5f7285',
  '#6c7885',
  '#8a6a4a',
  '#1e262e',
  '#bacede',
  '#8b97a3',
  '#cfd6dc',
]);

const COLOUR = /#[0-9a-fA-F]{3,8}\b|rgba?\([^)]*\)|hsla?\([^)]*\)|oklch\([^)]*\)|\boklab\([^)]*\)/g;
// The leading-dot form has to be part of the number, not skipped over: CSS is written `.3s`
// far more often than `0.3s`, and reading that as `3s` rejects 300ms — a duration §11.6 permits.
// `.` joins the lookbehind so the digits after a dot can never be matched on their own.
const DURATION = /(?<![\w.-])(\d+(?:\.\d+)?|\.\d+)(ms|s)(?![\w-])/g;
// Longest first, or `ease-in-out` is reported as `ease` and the message names a value the
// stylesheet does not contain.
const TIMING =
  /cubic-bezier\([^)]*\)|\b(?:ease-in-out|ease-in|ease-out|ease|linear|steps\([^)]*\))\b/g;
const KEYFRAMES = /@keyframes\s+([A-Za-z_][\w-]*)/g;

// A curve is a set of four numbers, not a spelling. The spec writes `cubic-bezier(.2,.85,.2,1)`
// and a CSS formatter writes `cubic-bezier(0.2, 0.85, 0.2, 1)`; comparing the raw strings makes
// the gate fail on whitespace and a leading zero, which says nothing about §11.6.
const curveKey = (v) => v.replace(/\s+/g, '').replace(/\b0\.(\d)/g, '.$1');

// And a colour is its channels and its alpha, not its spelling — the same argument as `curveKey`
// one line up, which was applied to curves and not to colours only because no stylesheet carried
// an `rgb()` literal until the card did. A CSS formatter writes `rgb(255 255 255 / 0.13)` where
// §7.7's band table writes `/ .13`; rejecting that fails 20 exempt colours over a leading zero
// and says nothing about criterion 46. A different alpha still fails, which is the point.
const colourKey = (v) =>
  v
    .replace(/\s+/g, ' ')
    .replace(/\b0\.(\d)/g, '.$1')
    .toLowerCase();

const exemptColours = new Set(vocabulary.exemptColourLiterals.map(colourKey));
const permittedDurations = new Set(vocabulary.durationsMs);
const permittedTimings = new Set(vocabulary.timingFunctions.map(curveKey));

// R35(b). A duplicate @keyframes of the same name silently overrides rather than erroring, and a
// keyframe declared inside a screen's stylesheet is invisible to motion.css's tier clamp.
const sharedKeyframes = new Set(vocabulary.sharedKeyframes);
const baseFile = join(rendererDir, 'styles/base.css');
const keyframeSites = new Map(); // name -> [file, …]

const scanned = cssFiles(rendererDir);

for (const file of scanned) {
  const text = readFileSync(file, 'utf8');
  const isTokenFile = file === tokensFile;

  for (const [literal] of text.matchAll(COLOUR)) {
    const normalised = colourKey(literal);
    if (isTokenFile) continue;
    if (exemptColours.has(normalised)) continue;
    if (CONDITION_DOT_FILLS.has(normalised)) continue;
    fail(file, `colour literal outside the token block: ${literal}`);
  }

  if (!isTokenFile) {
    for (const [, amount, unit] of text.matchAll(DURATION)) {
      const ms = unit === 's' ? Number(amount) * 1000 : Number(amount);
      if (!permittedDurations.has(ms)) fail(file, `duration ${amount}${unit} is not in §11.6`);
    }
    for (const [timing] of text.matchAll(TIMING)) {
      if (!permittedTimings.has(curveKey(timing))) {
        fail(file, `timing function not in §11.6: ${timing}`);
      }
    }
  }

  for (const [, name] of text.matchAll(/var\(--([a-z0-9-]+)/g)) {
    // Card-local properties are set inline per project and carry the jewel; they are exempt
    // by name in criterion 46 and are namespaced so the gate can tell them apart.
    if (name.startsWith('cdt-')) continue;
    if (!tokenNames.has(name)) fail(file, `var(--${name}) is not declared in tokens.css`);
  }

  for (const [, name] of text.matchAll(KEYFRAMES)) {
    if (!keyframeSites.has(name)) keyframeSites.set(name, []);
    keyframeSites.get(name).push(file);
  }
}

for (const [name, sites] of keyframeSites) {
  if (sites.length > 1) {
    const where = sites.map((f) => relative(root, f)).join(', ');
    fail(
      sites[0],
      `@keyframes ${name} is declared ${sites.length} times (${where}); the later one silently overrides`,
    );
  }
  if (sharedKeyframes.has(name) && sites.some((f) => f !== baseFile)) {
    fail(
      sites[0],
      `@keyframes ${name} is shared and belongs in styles/base.css (R35(b)); a screen-local copy is invisible to motion.css's tier clamp`,
    );
  }
}

for (const name of sharedKeyframes) {
  if (!keyframeSites.has(name)) fail(baseFile, `@keyframes ${name} is declared by nobody`);
}

// A gate whose passing run reads nothing is a failing gate that looks green. Two have already
// shipped here: a CI grep over gitignored paths, and a motion clamp asserting an attribute the
// product never sets. So the count is both enforced and printed.
if (scanned.length === 0) {
  console.error(`check-style-tokens: found no css file under ${relative(root, rendererDir)}`);
  process.exit(1);
}

const summary = `${String(scanned.length)} css file(s), ${String(keyframeSites.size)} keyframe(s)`;

if (failures.length > 0) {
  for (const line of failures) console.error(`check-style-tokens: ${line}`);
  console.error(`check-style-tokens: ${String(failures.length)} problem(s) over ${summary}`);
  process.exit(1);
}
console.error(`check-style-tokens: ok — ${summary}`);

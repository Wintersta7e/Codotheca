#!/usr/bin/env node
// §34.8's standing checker: every animated class in the renderer's stylesheets is clamped at
// `reduced` AND at `off`.
//
// An instance test proves one element is clamped; this proves the rule, so the NEXT class name
// cannot escape. `motion.css` enumerates class names and `reduced` has no blanket rule at all, so
// a new animated class is unclamped there by default and plays its full envelope to a user who
// asked their operating system for reduced motion. The class-name set is DERIVED here and printed
// with its size; no criterion states a count of it (R112).
//
// A class is clamped at a tier when a rule scoped to that tier names it as the element it styles,
// in any renderer stylesheet — `motion.css` holds the card family's clamps and two surfaces keep
// their own beside their animations. Two readings stand in for a named rule, each stated where it
// is decided below: an animation under `motion.css`'s `off` blanket, and a declaration already at
// or under `REDUCED_CLAMP_MS` at `reduced`. A rule scoped to `full` is absent below it by
// construction and clamps itself.
//
// Durations are compared as numbers; `REDUCED_CLAMP_MS` is read from `motion/tier.ts`, never
// restated. Stylesheets are read through `read-scanned.mjs`, which skips a file a parallel test
// removed BEFORE it is counted, and a run that scans no stylesheet or no animated selector fails.
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  cssRules,
  declarations,
  durationsMs,
  subjectClasses,
  tierOf,
  transitionDurationsMs,
} from './lib/motion-clamp.mjs';
import { readScannedFile } from './lib/read-scanned.mjs';

const GATE = 'check-motion-clamp';
const RESULT_ID = 'check-motion-clamp:every-animated-class-is-clamped';

/**
 * Animated classes this checker found unclamped when it landed, each with the reason. **The list
 * can only shrink**: an entry that stops escaping fails the run until it is removed, so it cannot
 * sit here after the defect behind it is fixed. A new name is never added to pass the gate.
 *
 * Empty now: the six it landed with were clamped in the stylesheets that own them, and the one
 * that only code gated — the rescan line's travelling band — is scoped to `full` in CSS too.
 */
export const KNOWN_ESCAPES = new Map();

/** `REDUCED_CLAMP_MS`, read out of the one file that declares it. */
export function reducedClampMs(tierSource) {
  const m = /export\s+const\s+REDUCED_CLAMP_MS\s*=\s*(\d+)/u.exec(tierSource);
  return m === null ? null : Number(m[1]);
}

function cssFiles(dir) {
  const out = [];
  if (!existsSync(dir)) return out;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...cssFiles(full));
    else if (entry.name.endsWith('.css')) out.push(full);
  }
  return out.sort();
}

/** What makes a rule animated, or `null`: an animation, or a transition longer than the clamp. */
function motionOf(body, clampMs) {
  let animation = false;
  let longest = 0;
  let infinite = false;
  let overCeiling = false;
  for (const { property, value } of declarations(body)) {
    if (property === 'animation' || property === 'animation-name') {
      if (/^none\b/iu.test(value)) continue;
      animation = true;
      if (/\binfinite\b/iu.test(value)) infinite = true;
      // `animation`'s first time value is its duration; a longhand name carries none.
      if (property === 'animation') longest = Math.max(longest, durationsMs(value)[0] ?? 0);
    } else if (property === 'animation-duration') {
      longest = Math.max(longest, ...durationsMs(value));
    } else if (property === 'animation-iteration-count' && /\binfinite\b/iu.test(value)) {
      infinite = true;
    } else if (property === 'transition') {
      const over = transitionDurationsMs(value).filter((ms) => ms > clampMs);
      if (over.length > 0) {
        overCeiling = true;
        longest = Math.max(longest, ...over);
      }
    } else if (property === 'transition-duration') {
      const over = durationsMs(value).filter((ms) => ms > clampMs);
      if (over.length > 0) {
        overCeiling = true;
        longest = Math.max(longest, ...over);
      }
    }
  }
  if (!animation && !overCeiling) return null;
  return { animation, transition: overCeiling, longest, infinite };
}

/**
 * Run the check over a tree laid out like this repository.
 *
 * Returns what it scanned and what failed rather than exiting, so the self-test can drive it over
 * a fixture tree. `known` is `KNOWN_ESCAPES` unless a fixture tree needs its own.
 */
export function checkMotionClamp(root, known = KNOWN_ESCAPES) {
  const rendererDir = join(root, 'app/src/renderer');
  const motionPath = join(rendererDir, 'styles/motion.css');
  const tierPath = join(rendererDir, 'motion/tier.ts');
  const failures = [];

  const tierSource = existsSync(tierPath) ? readFileSync(tierPath, 'utf8') : '';
  const clampMs = reducedClampMs(tierSource);
  if (clampMs === null) {
    failures.push(`${relative(root, tierPath)}: declares no REDUCED_CLAMP_MS to compare against`);
  }

  // Read first, count only what was read: a file a parallel test removed is not a file scanned.
  const sheets = [];
  for (const path of cssFiles(rendererDir)) {
    const text = readScannedFile(path);
    if (text === null) continue;
    sheets.push({ path, text });
  }

  // `motion.css`'s `off` blanket — `[data-effects-tier='off'] * { animation: none }` — is what
  // stops every animation at `off`. Derived from the file, so deleting it un-clamps every
  // animation this checker would otherwise pass.
  const motion = sheets.find((s) => s.path === motionPath);
  const offAnimationBlanket =
    motion !== undefined &&
    cssRules(motion.text).some(
      (rule) =>
        rule.selectors.some((s) =>
          /^\[data-effects-tier\s*=\s*['"]?off['"]?\s*\]\s+\*$/u.test(s),
        ) &&
        declarations(rule.body).some(
          (d) => d.property === 'animation' && /^none\b/iu.test(d.value),
        ),
    );

  // Every class a tier-scoped rule names as the element it styles, per tier.
  const named = { reduced: new Set(), off: new Set() };
  for (const sheet of sheets) {
    for (const rule of cssRules(sheet.text)) {
      for (const selector of rule.selectors) {
        const tier = tierOf(selector);
        if (tier !== 'reduced' && tier !== 'off') continue;
        for (const name of subjectClasses(selector)) named[tier].add(name);
      }
    }
  }

  const subjects = [];
  for (const sheet of sheets) {
    for (const rule of cssRules(sheet.text)) {
      const motionFacts = clampMs === null ? null : motionOf(rule.body, clampMs);
      if (motionFacts === null) continue;
      for (const selector of rule.selectors) {
        const tier = tierOf(selector);
        // A rule scoped to a tier is either the clamp itself (`reduced`, `off`) or absent below
        // `full` by construction; neither is an escape.
        if (tier !== null) continue;
        subjects.push({ file: relative(root, sheet.path), selector, motion: motionFacts });
      }
    }
  }

  const escapes = new Map();
  let matched = 0;
  for (const subject of subjects) {
    const classes = subjectClasses(subject.selector);
    if (classes.length === 0) {
      failures.push(
        `${subject.file}: \`${subject.selector}\` animates an element no class names, so no ` +
          'clamp can select it',
      );
      continue;
    }
    const { animation, transition, longest, infinite } = subject.motion;
    // At or under the ceiling, and not looping, a declaration is inside the clamp already.
    const withinCeiling = clampMs !== null && !infinite && longest <= clampMs;
    const atReduced = classes.some((c) => named.reduced.has(c)) || withinCeiling;
    const atOff =
      classes.some((c) => named.off.has(c)) || (animation && !transition && offAnimationBlanket);
    if (atReduced && atOff) {
      matched += 1;
      continue;
    }
    const missing = [atReduced ? null : 'reduced', atOff ? null : 'off'].filter(Boolean);
    for (const name of classes) {
      const list = escapes.get(name) ?? [];
      list.push(
        `${subject.file}: \`${subject.selector}\` is unclamped at ${missing.join(' and ')}`,
      );
      escapes.set(name, list);
    }
  }

  const excused = [];
  for (const [name, where] of escapes) {
    if (known.has(name)) excused.push(name);
    else failures.push(...where);
  }
  for (const name of known.keys()) {
    if (!escapes.has(name)) {
      failures.push(`KNOWN_ESCAPES still lists \`.${name}\`, which is clamped now — remove it`);
    }
  }

  if (sheets.length === 0) failures.push('scanned no stylesheet, so it proved nothing');
  if (subjects.length === 0) failures.push('found no animated selector, so it proved nothing');

  return {
    clampMs,
    sheets: sheets.length,
    scanned: subjects.length,
    matched,
    known: excused,
    clampSet: new Set([...named.reduced, ...named.off]),
    failures,
  };
}

function main(argv) {
  const at = argv.indexOf('--root');
  const root =
    at === -1 ? join(dirname(fileURLToPath(import.meta.url)), '..') : (argv[at + 1] ?? '.');
  const result = checkMotionClamp(root);
  const set = [...result.clampSet].sort();
  console.error(
    `${GATE}: REDUCED_CLAMP_MS ${String(result.clampMs)}; clamp set (${String(set.length)}): ` +
      set.join(' '),
  );
  console.error(
    `${GATE}: ${String(result.sheets)} stylesheets, scanned ${String(result.scanned)} animated ` +
      `selectors, matched ${String(result.matched)}, known escapes ${String(result.known.length)}` +
      (result.known.length > 0 ? ` (${result.known.join(' ')})` : ''),
  );
  for (const failure of result.failures) console.error(`${GATE}: ${failure}`);

  const out = join(root, 'acceptance/results');
  if (existsSync(out)) {
    const status = result.failures.length === 0 ? 'passed' : 'failed';
    const detail = result.failures.length === 0 ? '' : result.failures.join('\n');
    writeFileSync(
      join(out, 'script-motionclamp.json'),
      `${JSON.stringify([{ id: RESULT_ID, status, detail }], null, 2)}\n`,
    );
  }
  return result.failures.length === 0 ? 0 : 1;
}

if (process.argv[1] !== undefined && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exitCode = main(process.argv.slice(2));
}

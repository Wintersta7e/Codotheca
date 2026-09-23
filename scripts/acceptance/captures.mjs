#!/usr/bin/env node
/**
 * Who writes `acceptance/results/`, and whether the gate runs every one of them.
 *
 * `npm run acceptance` runs no suite: it grades whatever the capture files hold, so a writer the
 * gate never runs leaves rows as old as the last hand run, and the register is graded against a
 * tree nobody tested. The number of writers has been stated four different ways and was wrong each
 * time, so it is not stated here — it is derived, printed, and compared by name.
 *
 * The rule: **every capture file the CLI parses has a gate step that writes it.**
 *
 *   node scripts/acceptance/captures.mjs --gate <gate script>
 *
 * Exits 1 when a declared writer has no gate step, when the tree and the declaration disagree, or
 * when the scan found nothing; exits 2 on a usage error rather than passing on no input.
 */
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { readScannedFile } from '../lib/read-scanned.mjs';

/**
 * Every capture writer, by the file it writes. A runner's capture is written by the gate itself
 * (a copy or a reporter flag), so it names no npm script; a static gate's is written by the script
 * an npm script runs.
 */
export const CAPTURE_WRITERS = [
  { file: 'cargo.txt', writer: 'cargo test, copied by the gate', npmScript: null },
  { file: 'e2e.json', writer: 'playwright, copied by the gate', npmScript: null },
  {
    file: 'node-harness.json',
    writer: 'node:test, through scripts/acceptance/node-reporter.mjs',
    npmScript: null,
  },
  {
    file: 'node-protocol.json',
    writer: 'node:test, through scripts/acceptance/node-reporter.mjs',
    npmScript: null,
  },
  { file: 'vitest.json', writer: "vitest's JSON reporter", npmScript: null },
  { file: 'script-bundle.json', writer: 'scripts/check-bundle.mjs', npmScript: 'check:bundle' },
  {
    file: 'script-callsites.json',
    writer: 'scripts/check-call-sites.mjs',
    npmScript: 'check:callsites',
  },
  {
    file: 'script-forbidden.json',
    writer: 'scripts/check-forbidden.mjs',
    npmScript: 'check:forbidden',
  },
  {
    file: 'script-motionclamp.json',
    writer: 'scripts/check-motion-clamp.mjs',
    npmScript: 'lint:clamp',
  },
  {
    file: 'script-problems.json',
    writer: 'scripts/check-problem-kinds.mjs',
    npmScript: 'check:problems',
  },
];

const RUNNER_WRITERS = new Map(
  CAPTURE_WRITERS.filter((w) => w.npmScript === null).map((w) => [w.file, w.writer]),
);

/**
 * The writers the tree actually holds. The runner captures are the exact names the CLI parses
 * (`collectResults`, `scripts/acceptance.mjs`); the static ones are every `script-*.json` a
 * top-level script writes, joined to the npm script that runs it. Files are read through
 * `read-scanned.mjs` and a vanished one is skipped before it is counted.
 */
export function writersInTree(root) {
  const rows = [];
  let scanned = 0;
  const cli = existsSync(join(root, 'scripts/acceptance.mjs'))
    ? readScannedFile(join(root, 'scripts/acceptance.mjs'))
    : null;
  if (cli !== null) {
    scanned += 1;
    for (const m of cli.matchAll(/name === '([a-z0-9.-]+\.(?:txt|json))'/gu)) {
      rows.push({ file: m[1], writer: RUNNER_WRITERS.get(m[1]) ?? 'undeclared', npmScript: null });
    }
  }
  const scripts = existsSync(join(root, 'scripts'))
    ? readdirSync(join(root, 'scripts')).filter((f) => f.endsWith('.mjs') && !f.includes('.test.'))
    : [];
  const pkgPath = join(root, 'package.json');
  const npmScripts = existsSync(pkgPath)
    ? Object.entries(JSON.parse(readFileSync(pkgPath, 'utf8')).scripts ?? {})
    : [];
  for (const name of scripts) {
    const text = readScannedFile(join(root, 'scripts', name));
    if (text === null) continue;
    scanned += 1;
    // Either spelling a writer uses: `join(out, 'script-x.json')` or a path ending in it.
    for (const m of text.matchAll(/['"`/](script-[a-z0-9-]+\.json)['"`]/gu)) {
      const writer = `scripts/${name}`;
      const runs = npmScripts.find(([, command]) => String(command).includes(`node ${writer}`));
      rows.push({ file: m[1], writer, npmScript: runs === undefined ? null : runs[0] });
    }
  }
  if (scanned === 0) throw new Error(`captures: scanned nothing under ${root}`);
  return { rows, scanned };
}

/**
 * The root-package scripts `text` runs. `npm run <name> --workspace <w>` runs a script of another
 * package's `package.json`, which declares no capture writer, so it is not followed.
 */
function npmRuns(text) {
  const names = [];
  for (const m of String(text).matchAll(/\bnpm run ([a-z0-9:_-]+)(\s+--workspace\b)?/gu)) {
    if (m[2] === undefined) names.push(m[1]);
  }
  return names;
}

/** The live lines of a gate script: a comment line is not a step. */
const liveLines = (gateText) =>
  gateText
    .split('\n')
    .filter((line) => !line.trimStart().startsWith('#'))
    .join('\n');

/**
 * The declared writers whose capture no gate step produces: a runner capture needs a live line
 * naming `acceptance/results/<file>`; a static one needs its npm script **reachable** from a
 * script the gate runs, through `package.json`'s script graph — `lint` runs `lint:shell`, which
 * runs `lint:clamp`, so a gate running `npm run lint` writes `script-motionclamp.json`.
 */
export function stepsMissingFrom(gateText, packageJson, writers = CAPTURE_WRITERS) {
  const scripts = packageJson.scripts ?? {};
  const live = liveLines(gateText);
  const reached = new Set();
  const pending = npmRuns(live);
  while (pending.length > 0) {
    const name = pending.pop();
    if (reached.has(name) || scripts[name] === undefined) continue;
    reached.add(name);
    pending.push(...npmRuns(scripts[name]));
  }
  const missing = [];
  for (const w of writers) {
    if (w.npmScript === null) {
      if (!live.includes(`acceptance/results/${w.file}`)) missing.push(w.file);
    } else if (!reached.has(w.npmScript)) {
      missing.push(w.npmScript);
    }
  }
  return missing;
}

function main(argv) {
  const at = argv.indexOf('--gate');
  const gatePath = at === -1 ? undefined : argv[at + 1];
  if (gatePath === undefined) {
    console.error('usage: captures.mjs --gate <gate script>');
    return 2;
  }
  const root = join(dirname(fileURLToPath(import.meta.url)), '../..');
  const { rows, scanned } = writersInTree(root);
  const key = (r) => `${r.file} <- ${r.writer} (${String(r.npmScript)})`;
  const declared = new Set(CAPTURE_WRITERS.map(key));
  const derived = new Set(rows.map(key));
  const undeclared = [...derived].filter((k) => !declared.has(k));
  const unwritten = [...declared].filter((k) => !derived.has(k));
  const missing = stepsMissingFrom(
    readFileSync(gatePath, 'utf8'),
    JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')),
  );
  const n = CAPTURE_WRITERS.length;
  console.error(
    `captures: ${String(n)} writers, ${String(n - missing.length)} with a gate step ` +
      `(${String(scanned)} files scanned)`,
  );
  for (const k of undeclared) console.error(`captures: written in the tree, not declared: ${k}`);
  for (const k of unwritten) console.error(`captures: declared, written by nothing: ${k}`);
  for (const m of missing) console.error(`captures: no gate step writes ${m}`);
  const ok = n > 0 && missing.length === 0 && undeclared.length === 0 && unwritten.length === 0;
  return ok ? 0 : 1;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) process.exit(main(process.argv.slice(2)));

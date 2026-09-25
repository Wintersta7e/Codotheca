#!/usr/bin/env node
/**
 * The acceptance gate.
 *
 *   npm run acceptance                     validate, join whatever ran, gate, write the report
 *   npm run acceptance -- --dispositions   regenerate the committed disposition table
 *   npm run acceptance -- --allow-empty    join an empty results directory without failing
 *
 * The suites write into acceptance/results/; this script reads the directory. A partial run is
 * representable and says which suites were absent, instead of reporting every absent suite as a
 * wall of deleted tests.
 *
 * Exit 1 means the gate found problems. Exit 2 means the gate could not run — which is also a
 * failure, and includes a results directory with nothing in it. A gate whose passing run read
 * zero results is not a gate.
 */
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  absentRunners,
  diffAgainstBaseline,
  gateProblems,
  joinResults,
  recordCounts,
  validateBaseline,
} from './acceptance/join.mjs';
import { loadRegistry, validatePhase2Complete, validateRegistry } from './acceptance/registry.mjs';
import {
  parseLibtest,
  parseNodeTest,
  parsePlaywright,
  parseScriptResults,
  parseVitest,
} from './acceptance/runners.mjs';
import { renderDispositions, renderRegistryLine, renderRunReport } from './acceptance/report.mjs';

const EXIT_PROBLEMS = 1;
const EXIT_CANNOT_RUN = 2;

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const registryPath = join(root, 'acceptance/criteria.json');
const baselinePath = join(root, 'acceptance/baseline.json');
const resultsDir = join(root, 'acceptance/results');

export function collectResults(dir) {
  if (!existsSync(dir)) return [];
  const out = [];
  for (const name of readdirSync(dir).sort()) {
    const full = join(dir, name);
    const text = readFileSync(full, 'utf8');
    if (name === 'cargo.txt') out.push(...parseLibtest(text));
    else if (extname(name) !== '.json') continue;
    else if (name === 'vitest.json') out.push(...parseVitest(JSON.parse(text)));
    else if (name === 'e2e.json') out.push(...parsePlaywright(JSON.parse(text)));
    // The two `node:test` suites run as two gate steps, so each writes a capture of its own
    // rather than overwriting one.
    else if (name === 'node-harness.json') out.push(...parseNodeTest(JSON.parse(text)));
    else if (name === 'node-protocol.json') out.push(...parseNodeTest(JSON.parse(text)));
    else if (name.startsWith('script-')) out.push(...parseScriptResults(JSON.parse(text)));
  }
  return out;
}

const git = (root, args) =>
  execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });

/**
 * The commit this run grades, or `null` outside a git checkout. A manual record binds to the tree
 * it ran against (§49.6), so a run that cannot name its own commit cannot count one.
 */
export function gradedCommit(root) {
  try {
    return git(root, ['rev-parse', 'HEAD']).trim();
  } catch {
    return null;
  }
}

const diffs = new Map();

/**
 * `git diff --name-only <from> <to>`, or `null` when `from` is not in this clone — CI checks out
 * one commit deep, and a record whose commit it cannot see cannot be shown to count. Memoised per
 * pair: every record made against one commit reads one diff.
 */
export function changedFiles(root, from, to) {
  const key = `${root}\0${String(from)}\0${String(to)}`;
  if (!diffs.has(key)) {
    let files = null;
    try {
      git(root, ['cat-file', '-e', `${String(from)}^{commit}`]);
      files = git(root, ['diff', '--name-only', String(from), String(to)])
        .split('\n')
        .filter((line) => line.length > 0);
    } catch {
      files = null;
    }
    diffs.set(key, files);
  }
  return diffs.get(key);
}

/** How a manual check's record stands at `graded`, for `joinResults`. */
export function recordStatusAt(root, graded) {
  return (check) => {
    const record = check.record ?? null;
    const changed =
      record === null || graded === null ? [] : changedFiles(root, record.commit, graded);
    return recordCounts(record, graded, changed);
  };
}

/**
 * The rule files, for the surviving-escape half of the completeness audit.
 *
 * Read here rather than inside the validator so a unit test can validate a registry object with
 * no I/O, and so a missing rule file is a *problem* the audit reports rather than an exception
 * that reads as a crash.
 */
function staticRules() {
  const read = (name) => {
    const path = join(root, `acceptance/${name}`);
    return existsSync(path) ? JSON.parse(readFileSync(path, 'utf8')) : { rules: [] };
  };
  return { callsites: read('callsites.json'), forbidden: read('forbidden.json') };
}

function main(argv) {
  const registry = loadRegistry(registryPath);
  const problems = validateRegistry(registry, root);
  // R46, mechanically: a phase-2 criterion deferred to a plan that has merged, a static rule
  // still carrying the escape a registered check discharges, or a join key two checks claim
  // without declaring it. Each is a way a criterion ships as an intention and the gate reports
  // a clean run over it.
  problems.push(...validatePhase2Complete(registry, staticRules()));
  if (problems.length > 0) {
    for (const p of problems) console.error(`acceptance: registry: ${p}`);
    return EXIT_CANNOT_RUN;
  }

  // What it validated, per phase. A register validator that validated nothing and said nothing
  // is the gate-that-cannot-fail shape one level inside the gate built to catch it.
  //
  // The zero-criteria refusal is NOT repeated here: `validateRegistry` pushes
  // `registry.criteria is empty` (`acceptance/registry.mjs`) and this function has already
  // returned EXIT_CANNOT_RUN above on it. A `total === 0` branch below that line is
  // unreachable — one value stated twice, with the second copy dead and untestable, which is
  // the defect the register exists to catch wearing a reassuring shape.
  console.error(`acceptance: registry: ${renderRegistryLine(registry)}`);

  if (argv.includes('--dispositions')) {
    writeFileSync(join(root, 'acceptance/DISPOSITIONS.md'), renderDispositions(registry));
    console.error('acceptance: wrote acceptance/DISPOSITIONS.md');
    return 0;
  }

  const baseline = JSON.parse(readFileSync(baselinePath, 'utf8'));
  const baselineProblems = validateBaseline(baseline, registry);
  if (baselineProblems.length > 0) {
    for (const p of baselineProblems) console.error(`acceptance: baseline: ${p}`);
    return EXIT_CANNOT_RUN;
  }

  const results = collectResults(resultsDir);
  if (results.length === 0 && !argv.includes('--allow-empty')) {
    console.error(
      'acceptance: acceptance/results/ holds no runner output — run the suites first.\n' +
        'A gate that read zero results reports zero failures for the wrong reason.',
    );
    return EXIT_CANNOT_RUN;
  }

  const joined = joinResults(registry, results, recordStatusAt(root, gradedCommit(root)));
  const diff = diffAgainstBaseline(results, baseline);
  const absent = absentRunners(joined, results);
  const gate = gateProblems(joined, diff, results);

  mkdirSync(resultsDir, { recursive: true });
  writeFileSync(
    join(root, 'acceptance/report.md'),
    renderRunReport(registry, joined, diff, [], absent),
  );
  writeFileSync(
    join(root, 'acceptance/report.json'),
    `${JSON.stringify({ checks: joined.checks, diff, absent, problems: gate }, null, 2)}\n`,
  );

  for (const p of gate) console.error(`acceptance: ${p}`);
  if (absent.length > 0) {
    console.error(`acceptance: partial run — no results from: ${absent.join(', ')}`);
  }
  console.error(
    `acceptance: ${String(joined.checks.length)} checks, ${String(results.length)} results, ` +
      `${String(gate.length)} problems`,
  );
  return gate.length === 0 ? 0 : EXIT_PROBLEMS;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) process.exit(main(process.argv.slice(2)));

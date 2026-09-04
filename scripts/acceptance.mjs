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
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  absentRunners,
  diffAgainstBaseline,
  gateProblems,
  joinResults,
  validateBaseline,
} from './acceptance/join.mjs';
import { loadRegistry, validateRegistry } from './acceptance/registry.mjs';
import {
  parseLibtest,
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
    else if (name.startsWith('script-')) out.push(...parseScriptResults(JSON.parse(text)));
  }
  return out;
}

function main(argv) {
  const registry = loadRegistry(registryPath);
  const problems = validateRegistry(registry, root);
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

  const joined = joinResults(registry, results);
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

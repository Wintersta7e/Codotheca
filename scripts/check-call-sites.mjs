#!/usr/bin/env node
/**
 * The zero-call-sites gate.
 *
 * Two commands and one column ship with no phase-1 caller by ruling, not by omission; two
 * further rules assert that a control is not wired to the wrong command and that no git
 * invocation writes the user's config. A reference outside the allowed files fails the build,
 * which is the point: the failure mode is a later reader wiring a control to a name that looks
 * unfinished.
 *
 * Exit 1 = a call site was found. Exit 2 = the gate could not run, which is also a failure.
 */
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, extname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import { readScannedFile } from './lib/read-scanned.mjs';
import { loadRegistry } from './acceptance/registry.mjs';
import {
  EXIT_CANNOT_RUN,
  EXIT_VIOLATION,
  TEST_FILE,
  stripComments,
  stripRustTestModules,
} from './check-forbidden.mjs';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const GATE = 'check-call-sites';

const posix = (path) => path.split(sep).join('/');

function* filesUnder(dir, extensions) {
  if (!existsSync(dir)) return;
  for (const entry of readdirSync(dir).sort()) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) yield* filesUnder(full, extensions);
    else if (extensions.includes(extname(entry))) yield full;
  }
}

/**
 * Walk, then read. Both halves refuse an empty result: this gate is *already* asserting an
 * absence, so an empty input produces the exact output a passing run produces. That is the
 * specific way a zero-call-sites gate dies.
 */
function filesFor(root, rule) {
  const walked = [];
  for (const dir of rule.roots) walked.push(...filesUnder(join(root, dir), rule.ext));
  if (walked.length === 0) {
    throw new Error(
      `${GATE}: rule "${rule.id}" walked zero files under ${rule.roots.join(', ')} — ` +
        'a zero-call-sites check over zero files reports zero call sites for the wrong reason',
    );
  }
  const read = [];
  for (const file of walked) {
    const rel = posix(relative(root, file));
    if (rule.allow.some((pattern) => rel.includes(pattern))) continue;
    if (TEST_FILE.test(rel)) continue;
    const raw = readScannedFile(file);
    if (raw === null) continue;
    const ext = extname(file);
    let text = stripComments(raw, ext);
    if (ext === '.rs') text = stripRustTestModules(text);
    read.push({ path: rel, text });
  }
  if (read.length === 0) {
    throw new Error(
      `${GATE}: rule "${rule.id}" read zero of the ${String(walked.length)} files it walked — ` +
        'every file was excluded, so the rule asserts nothing',
    );
  }
  return read;
}

export function loadCallSites(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

export function validateCallSites(rules, registry) {
  const problems = [];
  const declared = new Set();
  for (const rule of rules.rules ?? []) {
    declared.add(rule.id);
    if (!Array.isArray(rule.roots) || rule.roots.length === 0)
      problems.push(`${rule.id}: needs roots`);
    if (!Array.isArray(rule.patterns) || rule.patterns.length === 0) {
      problems.push(`${rule.id}: needs patterns`);
    }
    if (typeof rule.why !== 'string' || rule.why.length < 20) {
      problems.push(`${rule.id}: why must say what the rule protects`);
    }
    if (
      rule.pendingRegistryEntry !== undefined &&
      typeof rule.pendingRegistryEntry.criterion !== 'string'
    ) {
      problems.push(`${rule.id}: a pending rule names the criterion it belongs to`);
    }
  }
  const claimed = new Set();
  for (const entry of registry.criteria ?? []) {
    for (const check of entry.checks ?? []) {
      if (typeof check.test !== 'string' || !check.test.startsWith(`${GATE}:`)) continue;
      const id = check.test.slice(GATE.length + 1);
      claimed.add(id);
      if (!declared.has(id))
        problems.push(`${check.id}: names ${check.test} and no rule implements it`);
    }
  }
  // The other direction, as `check-forbidden` does it: a rule nothing claims is a rule nobody
  // reads, unless it says out loud that its registry entry has not been written yet.
  for (const rule of rules.rules ?? []) {
    if (claimed.has(rule.id) || rule.pendingRegistryEntry !== undefined) continue;
    problems.push(`${rule.id}: no check in criteria.json claims ${GATE}:${rule.id}`);
  }
  return problems;
}

export function evaluateCallSites(root, rules) {
  const results = [];
  for (const rule of rules.rules) {
    const hits = [];
    const files = filesFor(root, rule);
    for (const { path, text } of files) {
      if (rule.mode === 'co-occurrence') {
        if (rule.patterns.every((p) => text.includes(p))) {
          hits.push(
            `${path}: contains ${rule.patterns.map((p) => JSON.stringify(p)).join(' and ')}`,
          );
        }
        continue;
      }
      text.split('\n').forEach((line, i) => {
        for (const source of rule.patterns) {
          if (!new RegExp(source, 'u').test(line)) continue;
          if ((rule.allowLines ?? []).some((x) => new RegExp(x, 'u').test(line))) continue;
          hits.push(`${path}:${i + 1}: ${line.trim()}`);
        }
      });
    }
    results.push({
      id: `${GATE}:${rule.id}`,
      status: hits.length === 0 ? 'passed' : 'failed',
      detail:
        hits.length === 0
          ? `${rule.spec} — no call site over ${String(files.length)} files`
          : hits.join('\n'),
      scanned: files.length,
    });
  }
  return results;
}

function main() {
  const rules = loadCallSites(join(ROOT, 'acceptance/callsites.json'));
  const problems = validateCallSites(rules, loadRegistry(join(ROOT, 'acceptance/criteria.json')));
  if (problems.length > 0) {
    for (const p of problems) console.error(`${GATE}: ${p}`);
    return EXIT_CANNOT_RUN;
  }
  let results;
  try {
    results = evaluateCallSites(ROOT, rules);
  } catch (error) {
    console.error(String(error instanceof Error ? error.message : error));
    return EXIT_CANNOT_RUN;
  }
  const out = join(ROOT, 'acceptance/results');
  if (existsSync(out)) {
    const rows = results.map(({ id, status, detail }) => ({ id, status, detail }));
    writeFileSync(join(out, 'script-callsites.json'), `${JSON.stringify(rows, null, 2)}\n`);
  }
  const failed = results.filter((r) => r.status === 'failed');
  for (const row of failed) console.error(`${row.id}\n${row.detail}`);
  const scanned = results.reduce((n, r) => n + r.scanned, 0);
  console.error(
    `${GATE}: ${String(results.length)} rules, ${String(failed.length)} violated, ` +
      `${String(scanned)} file reads`,
  );
  return failed.length === 0 ? 0 : EXIT_VIOLATION;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) process.exit(main());

#!/usr/bin/env node
/**
 * The banned-vocabulary gate.
 *
 * Rules live in acceptance/forbidden.json; this file only runs them. Each rule reports under the
 * check id acceptance/criteria.json already names for it, so a grep over a built bundle joins to
 * a criterion exactly like a unit test does.
 *
 * Two refusals are the point of the file. A rule whose target set resolves to zero files exits 2
 * — the dead in-sync CI step this repository shipped grepped two gitignored paths and was never
 * capable of failing. And a registry check id with no rule, or a rule no check claims, is a
 * validation error, so neither half can drift away from the other unnoticed.
 */
import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, extname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import { readScannedFile } from './lib/read-scanned.mjs';
import { loadRegistry } from './acceptance/registry.mjs';

export const FORBIDDEN_KINDS = ['literal', 'regex', 'co-occurrence', 'delegate'];
export const EXIT_VIOLATION = 1;
export const EXIT_CANNOT_RUN = 2;

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const GATE = 'check-forbidden';

function* filesUnder(dir, extensions) {
  if (!existsSync(dir)) return;
  for (const entry of readdirSync(dir).sort()) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) yield* filesUnder(full, extensions);
    else if (extensions.length === 0 || extensions.includes(extname(entry))) yield full;
  }
}

const posix = (path) => path.split(sep).join('/');

/**
 * Resolve one named target set to concrete files. Throws when it is empty: a rule that scanned
 * nothing has not passed, whatever its result says.
 */
export function resolveTargets(root, targets, name) {
  const spec = targets[name];
  if (spec === undefined) throw new Error(`${GATE}: unknown target set "${name}"`);
  const found = [];
  for (const entry of spec.dirs ?? []) found.push(...filesUnder(join(root, entry), spec.ext ?? []));
  for (const entry of spec.files ?? []) {
    const full = join(root, entry);
    if (existsSync(full)) found.push(full);
  }
  const kept = found.filter(
    (f) => !(spec.exclude ?? []).some((x) => posix(relative(root, f)).includes(x)),
  );
  if (kept.length === 0) {
    throw new Error(
      `${GATE}: target set "${name}" resolved to zero files (${spec.why ?? 'no build output?'}) — ` +
        'a gate that scans nothing cannot fail and is not a gate',
    );
  }
  return kept;
}

export function loadForbidden(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

/** Both directions, so neither the rule file nor the registry can quietly lose a rule. */
export function validateForbidden(rules, registry) {
  const problems = [];
  if (rules.version !== 1) problems.push('forbidden.version must be 1');
  const declared = new Set();
  for (const rule of rules.rules ?? []) {
    const where = String(rule.id);
    if (declared.has(rule.id)) problems.push(`${where}: duplicate rule id`);
    declared.add(rule.id);
    if (!FORBIDDEN_KINDS.includes(rule.kind)) {
      problems.push(`${where}: kind is not one of ${FORBIDDEN_KINDS.join(', ')}`);
    }
    if (typeof rule.why !== 'string' || rule.why.length < 20) {
      problems.push(`${where}: why must say what the rule protects`);
    }
    if (typeof rule.spec !== 'string' || !rule.spec.startsWith('§')) {
      problems.push(`${where}: spec must cite a section`);
    }
    if (rule.kind === 'delegate') {
      if (typeof rule.script !== 'string') {
        problems.push(`${where}: a delegate names the script that owns the rule`);
      }
      if (typeof rule.owner !== 'string') {
        problems.push(`${where}: a delegate names the plan that owns the script`);
      }
    } else if (!Array.isArray(rule.targets) || rule.targets.length === 0) {
      problems.push(`${where}: needs at least one target set`);
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
  for (const id of declared) {
    if (!claimed.has(id)) problems.push(`${id}: no check in criteria.json claims ${GATE}:${id}`);
  }
  return problems;
}

/**
 * Read the walked files, skipping one that vanished between the walk and the read **before**
 * counting it — the convention every gate under `scripts/` follows, so the printed count stays a
 * true count of files actually examined.
 */
function readAll(root, files) {
  const read = [];
  for (const file of files) {
    const text = readScannedFile(file);
    if (text === null) continue;
    read.push({ path: posix(relative(root, file)), text });
  }
  return read;
}

function scanLiteral(rule, files) {
  const hits = [];
  for (const { path, text } of files) {
    text.split('\n').forEach((line, i) => {
      for (const needle of rule.patterns) {
        if (!line.includes(needle)) continue;
        if ((rule.exempt ?? []).some((x) => line.includes(x))) continue;
        hits.push(`${path}:${i + 1}: contains ${JSON.stringify(needle)}`);
      }
    });
  }
  return hits;
}

function scanRegex(rule, files) {
  const hits = [];
  for (const { path, text } of files) {
    text.split('\n').forEach((line, i) => {
      for (const source of rule.patterns) {
        if (!new RegExp(source, rule.flags ?? 'u').test(line)) continue;
        if ((rule.exempt ?? []).some((x) => line.includes(x))) continue;
        hits.push(`${path}:${i + 1}: matches /${source}/`);
      }
    });
  }
  return hits;
}

function scanCoOccurrence(rule, files) {
  const hits = [];
  for (const { path, text } of files) {
    if (rule.patterns.every((needle) => text.includes(needle))) {
      hits.push(
        `${path}: contains all of ${rule.patterns.map((p) => JSON.stringify(p)).join(', ')}`,
      );
    }
  }
  return hits;
}

function runDelegate(root, rule) {
  const script = join(root, rule.script);
  if (!existsSync(script)) {
    throw new Error(
      `${GATE}: ${rule.id} delegates to ${rule.script}, owned by plan ${rule.owner}, which does not exist yet`,
    );
  }
  const run = spawnSync(process.execPath, [script], { cwd: root, encoding: 'utf8' });
  const tail = (run.stderr || run.stdout || '').trim().split('\n').slice(-3).join(' / ');
  const detail = `${rule.script} exited ${String(run.status)}: ${tail}`;
  return run.status === 0 ? [] : [detail];
}

export function evaluateForbidden(root, rules) {
  const results = [];
  for (const rule of rules.rules) {
    let hits;
    let scanned = 0;
    if (rule.kind === 'delegate') {
      hits = runDelegate(root, rule);
      scanned = 1;
    } else {
      const walked = rule.targets.flatMap((name) => resolveTargets(root, rules.targets, name));
      const files = readAll(root, walked);
      if (files.length === 0) {
        throw new Error(
          `${GATE}: rule "${rule.id}" read zero of the ${String(walked.length)} files it walked — ` +
            'a gate that scans nothing cannot fail and is not a gate',
        );
      }
      scanned = files.length;
      if (rule.kind === 'literal') hits = scanLiteral(rule, files);
      else if (rule.kind === 'regex') hits = scanRegex(rule, files);
      else hits = scanCoOccurrence(rule, files);
    }
    results.push({
      id: `${GATE}:${rule.id}`,
      status: hits.length === 0 ? 'passed' : 'failed',
      detail:
        hits.length === 0
          ? `${rule.spec} — clean over ${String(scanned)} ${rule.kind === 'delegate' ? 'delegate' : 'files'}`
          : hits.join('\n'),
      scanned,
    });
  }
  return results;
}

function main() {
  const rules = loadForbidden(join(ROOT, 'acceptance/forbidden.json'));
  const registry = loadRegistry(join(ROOT, 'acceptance/criteria.json'));
  const problems = validateForbidden(rules, registry);
  if (problems.length > 0) {
    for (const p of problems) console.error(`${GATE}: ${p}`);
    return EXIT_CANNOT_RUN;
  }

  let results;
  try {
    results = evaluateForbidden(ROOT, rules);
  } catch (error) {
    console.error(String(error instanceof Error ? error.message : error));
    return EXIT_CANNOT_RUN;
  }

  const out = join(ROOT, 'acceptance/results');
  if (existsSync(out)) {
    const rows = results.map(({ id, status, detail }) => ({ id, status, detail }));
    writeFileSync(join(out, 'script-forbidden.json'), `${JSON.stringify(rows, null, 2)}\n`);
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

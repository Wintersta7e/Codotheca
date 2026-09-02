#!/usr/bin/env node
/**
 * Every problem kind the protocol declares is storable, ordered and rendered — and nothing else
 * is any of those things.
 *
 * §11.1 draws eight groups from three backing stores: six read `scan_problem`, deferred-slow
 * reads `project_job_state`, and ambiguous lineage reads `project WHERE ambiguous_lineage = 1`.
 * Those two are declared here so their absence from the storage set is a statement rather than a
 * gap.
 *
 * The value of this gate is that no type system spans it: the vocabulary crosses a JSON schema,
 * a SQL CHECK constraint, a Rust match and a TypeScript record, and a kind that appears in four
 * of the five is unreachable in exactly the same way as one that appears in none.
 */
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { EXIT_CANNOT_RUN, EXIT_VIOLATION, stripRustTestModules } from './check-forbidden.mjs';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const GATE = 'check-problem-kinds';

/** The two groups §11.1 sources from somewhere other than `scan_problem`. */
export const NON_SCAN_PROBLEM_KINDS = {
  deferred_slow: 'project_job_state',
  ambiguous_lineage: 'project.ambiguous_lineage',
};

function read(path, why) {
  const full = join(ROOT, path);
  if (!existsSync(full)) throw new Error(`${GATE}: ${path} is missing — ${why}`);
  const text = readFileSync(full, 'utf8');
  if (text.trim().length === 0) throw new Error(`${GATE}: ${path} is empty — ${why}`);
  return text;
}

function matchAll(text, source, group, flags = 'gu') {
  return [...text.matchAll(new RegExp(source, flags))].map((m) => m[group]);
}

/** Pull one named block out, so a second array in the same file cannot pollute the set. */
function block(text, source, path, what) {
  const m = new RegExp(source, 'u').exec(text);
  if (m === null)
    throw new Error(`${GATE}: ${path} no longer declares ${what} in the shape this gate reads`);
  return m[1];
}

const snake = (v) => v.replace(/([a-z0-9])([A-Z])/gu, '$1_$2').toLowerCase();

export function collectProblemKinds() {
  const schema = JSON.parse(
    read('protocol/schema/protocol.json', 'run `npm run gen` after the schema plan'),
  );
  // R17: the schema's enum key is `variants`, never `values`.
  const declared = schema.types?.ProblemKind?.variants;
  if (!Array.isArray(declared) || declared.length === 0) {
    throw new Error(
      `${GATE}: protocol.json declares no ProblemKind.variants (R17: the key is "variants")`,
    );
  }

  // SQL comments first: the CHECK carries a comment naming the values it must spell.
  const sql = read(
    'core/migrations/0005_scan_and_art.sql',
    'the schema plan writes the scan tables',
  ).replace(/--[^\r\n]*/gu, '');
  const check = block(
    sql,
    'CREATE\\s+TABLE\\s+scan_problem[\\s\\S]*?\\bkind\\b[\\s\\S]*?CHECK\\s*\\(\\s*kind\\s+IN\\s*\\(([\\s\\S]*?)\\)',
    'core/migrations/0005_scan_and_art.sql',
    "scan_problem's kind CHECK set",
  );
  const storage = matchAll(check, "'([a-z_0-9]+)'", 1);

  const rust = matchAll(
    stripRustTestModules(read('core/src/scan/mod.rs', 'the scan plan writes ScanProblemKind')),
    'Self::[A-Za-z0-9]+\\s*=>\\s*"([a-z_0-9]+)"',
    1,
  );

  const orderSource = read('core/src/surfaces/problems.rs', 'the surfaces plan writes GROUP_ORDER');
  const order = matchAll(
    block(
      orderSource,
      'GROUP_ORDER\\s*:\\s*\\[ProblemKind;\\s*\\d+\\]\\s*=\\s*\\[([\\s\\S]*?)\\];',
      'core/src/surfaces/problems.rs',
      'GROUP_ORDER',
    ),
    'ProblemKind::([A-Za-z0-9]+)',
    1,
  ).map(snake);

  const labels = matchAll(
    block(
      read('app/src/renderer/summary/copy.ts', 'the summary plan writes PROBLEM_GROUP_LABEL'),
      'PROBLEM_GROUP_LABEL[^=]*=\\s*\\{([\\s\\S]*?)\\};',
      'app/src/renderer/summary/copy.ts',
      'PROBLEM_GROUP_LABEL',
    ),
    '^\\s*([a-z_0-9]+):\\s*[\'"`]',
    1,
    'gmu',
  );

  // Each source has to have produced something. Four empty sets and one full one would report
  // "every kind is unreachable", which is loud; but an empty *protocol* set with everything
  // else empty too would report clean, and that is the shape this repository keeps shipping.
  for (const [name, list] of [
    ['protocol', declared],
    ['storage', storage],
    ['rust', rust],
    ['order', order],
    ['labels', labels],
  ]) {
    if (list.length === 0) {
      throw new Error(
        `${GATE}: the ${name} source yielded no kinds — the gate read nothing and cannot pass`,
      );
    }
  }

  return { protocol: declared, storage, rust, order, labels };
}

function missing(from, against) {
  return from.filter((k) => !against.includes(k)).sort();
}

export function evaluateProblemKinds(sources) {
  const { protocol, storage, rust, order, labels } = sources;
  const storable = protocol.filter((k) => NON_SCAN_PROBLEM_KINDS[k] === undefined);
  const findings = [];
  for (const [what, list] of [
    ['a declared kind renders no string', missing(protocol, labels)],
    ['a rendered label names no declared kind', missing(labels, protocol)],
    ['a declared kind cannot be stored in scan_problem', missing(storable, storage)],
    ['scan_problem accepts a value the protocol does not declare', missing(storage, protocol)],
    ['ScanProblemKind emits a value scan_problem rejects', missing(rust, storage)],
    ['ScanProblemKind emits a value the protocol does not declare', missing(rust, protocol)],
    ['a declared kind is absent from GROUP_ORDER', missing(protocol, order)],
    ['GROUP_ORDER carries a kind the protocol does not declare', missing(order, protocol)],
  ]) {
    if (list.length > 0) findings.push(`${what}: ${list.join(', ')}`);
  }
  if (order.length !== protocol.length) {
    findings.push(
      `GROUP_ORDER has ${String(order.length)} entries for ${String(protocol.length)} declared kinds`,
    );
  }

  return [
    {
      id: `${GATE}:reachable`,
      status: findings.length === 0 ? 'passed' : 'failed',
      detail:
        findings.length === 0
          ? `§16.25 — ${String(protocol.length)} kinds, all storable, ordered and rendered`
          : findings.join('\n'),
    },
  ];
}

function main() {
  let results;
  let sources;
  try {
    sources = collectProblemKinds();
    results = evaluateProblemKinds(sources);
  } catch (error) {
    console.error(String(error instanceof Error ? error.message : error));
    return EXIT_CANNOT_RUN;
  }
  const out = join(ROOT, 'acceptance/results');
  if (existsSync(out))
    writeFileSync(join(out, 'script-problems.json'), `${JSON.stringify(results, null, 2)}\n`);
  const failed = results.filter((r) => r.status === 'failed');
  for (const row of failed) console.error(`${row.id}\n${row.detail}`);
  console.error(
    `${GATE}: protocol ${String(sources.protocol.length)}, storage ${String(sources.storage.length)}, ` +
      `rust ${String(sources.rust.length)}, order ${String(sources.order.length)}, ` +
      `labels ${String(sources.labels.length)} — ` +
      `${failed.length === 0 ? 'every kind is reachable' : 'unreachable kinds found'}`,
  );
  return failed.length === 0 ? 0 : EXIT_VIOLATION;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) process.exit(main());

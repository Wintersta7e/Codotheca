/**
 * §36's audits: criterion 22's one value and criterion 30, which carries D9's gate (§36.6); the
 * earlier criteria phase 3 moves, diffed against the copy frozen before wave 1 (§36.3); and the
 * gates that stopped being true when a phase-3 body landed (§36.4).
 *
 * A failure here names the criterion or the file **and the plan that owes the move**. This plan
 * does not edit another section's criterion text or test body: a row that fails is reported to
 * the lane that landed its body without its bar.
 */
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { readScannedFile } from '../lib/read-scanned.mjs';
import { loadRegistry } from './registry.mjs';

const registryPath = fileURLToPath(new URL('../../acceptance/criteria.json', import.meta.url));
const repoRoot = fileURLToPath(new URL('../..', import.meta.url));

const criterion = (id) => {
  const entry = loadRegistry(registryPath).criteria.find((c) => c.id === id);
  assert.ok(entry, `criterion ${id} is registered`);
  return entry;
};

// R127.6 settles the value; §36.6 keeps the landing gated. They are two checks on two criteria,
// because the validator makes a budget and `unmeasurable` mutually exclusive on one check.
test('criterion 22 states the texture cap as ~165 MB, unmeasurable and ungated', () => {
  const cap = criterion('22').checks.find((k) => k.id === 'AC-22-gpu-residency');
  assert.ok(cap, 'criterion 22 carries no GPU residency check');
  assert.equal(cap.status, 'unmeasurable');
  assert.equal(cap.runner, 'none');
  assert.equal(cap.budget, undefined, 'a budget on the cap would claim something measures it');
  assert.equal(cap.source, '§7.6');
  assert.match(cap.assert, /\b165\b/u);
});

test('no check anywhere states 128 MB as a GPU figure', () => {
  const stale = loadRegistry(registryPath)
    .criteria.flatMap((c) => c.checks)
    .filter((k) => /\b128 ?MB\b/iu.test(String(k.assert)) && /\bGPU\b/u.test(String(k.assert)))
    .filter((k) => !/supersed/iu.test(String(k.assert)));
  assert.deepEqual(
    stale.map((k) => k.id),
    [],
  );
  // AC-22-art-cache used to say the GPU clause is tested nowhere; it now names the check that
  // states it.
  const art = criterion('22').checks.find((k) => k.id === 'AC-22-art-cache');
  assert.doesNotMatch(art.assert, /GPU texture clause is phase 3 and is not tested/u);
  assert.match(art.assert, /AC-22-gpu-residency/u);
});

// §36.6: the budget IS the gate, so it stays; the run grows the case D9 is about. The name carries
// no check id: a tag would read this audit of the register entry as coverage of scroll pacing.
test('the pacing check keeps its budget and measures the phase-3 card at tier full', () => {
  const pacing = criterion('30').checks.find((k) => k.id === 'AC-30-pacing');
  assert.ok(pacing, 'criterion 30 carries no pacing check');
  assert.equal(pacing.status, 'deferred');
  assert.equal(pacing.runner, 'perf');
  assert.deepEqual(
    pacing.budget.map((b) => [b.metric, b.op, b.value]),
    [['missedFraction', '<=', 0.01]],
  );
  assert.match(pacing.measurement.run, /\btier\b/u);
  assert.match(pacing.measurement.run, /\bfull\b/u);
  assert.match(pacing.measurement.run, /phase-3 card/u);
});

// ---------------------------------------------------------------------------------------------
// §36.3 — the bar and the body move together, checked against the record taken before wave 1.
// You cannot verify that something is unchanged without a record of what it was.
// ---------------------------------------------------------------------------------------------

const frozen = JSON.parse(
  readFileSync(fileURLToPath(new URL('../../acceptance/phase3-frozen.json', import.meta.url))),
);
const frozenEntry = (id) => {
  const entry = frozen.criteria.find((c) => c.id === id);
  assert.ok(entry, `criterion ${id} is not in the frozen copy`);
  return entry;
};
const checkOf = (entry, id) => {
  const check = entry.checks.find((k) => k.id === id);
  assert.ok(check, `${entry.id} carries no ${id}`);
  return check;
};

test('46, owed by p3-33: material layers lose the exemption; vents and plate stops keep it', () => {
  const was = checkOf(frozenEntry('46'), 'AC-46-colour').assert;
  const now = checkOf(criterion('46'), 'AC-46-colour').assert;
  assert.match(was, /everything the rasterizer computes/u, 'the frozen copy is the old exemption');
  assert.doesNotMatch(now, /everything the rasterizer computes/u);
  assert.match(now, /plate stops/u);
  assert.match(now, /vent/u);
  assert.match(now, /MATERIAL LAYERS ARE NO LONGER EXEMPT/u);
});

test('62, owed by p3-33: the fade clause is an invariant, not a phase pin, with the byte identity', () => {
  const entry = criterion('62');
  const fade = checkOf(entry, 'AC-62-fade').assert;
  assert.match(checkOf(frozenEntry('62'), 'AC-62-fade').assert, /\bphase 1\b/u);
  assert.doesNotMatch(fade, /\bphase 1\b/u, 'AC-62-fade still pins the fade clause to phase 1');
  assert.match(fade, /invariant/u, 'AC-62-fade does not state the clause as an invariant');
  // §33's byte-identity clause, by its content rather than one wording of it: over closing every
  // open debt item, scene_hash and the rendition bytes are identical and nothing re-renders.
  assert.ok(
    entry.checks.some(
      (k) =>
        /debt item/u.test(k.assert) &&
        /scene_hash/u.test(k.assert) &&
        /identical/u.test(k.assert) &&
        /re-render/u.test(k.assert),
    ),
    "criterion 62 carries §33's byte-identity clause",
  );
});

test('50, owed by p3-33 and p3-34: the class-name count is deleted, not incremented', () => {
  const was = checkOf(frozenEntry('50'), 'AC-50-zero-animation').assert;
  const now = checkOf(criterion('50'), 'AC-50-zero-animation').assert;
  assert.match(was, /selects ten class names exactly/u, 'the frozen copy is the counted form');
  // No enumerated class name and no claimed count of them: the set is derived, printed and
  // checked at zero by the test the assert names.
  assert.doesNotMatch(now, /\bcdt-[a-z]/u, 'the assert still enumerates the clamp set');
  assert.doesNotMatch(now, /selects \w+ class names/u, 'the assert still states the count');
  assert.match(now, /derives the set/u);
  assert.match(now, /tierClamp\.test\.ts|AC-P3-33-8/u);
});

test('21, owed by p3-34: the surge sits beside the specular sweep; a schedule is still banned', () => {
  const asserts = criterion('21').checks.map((k) => k.assert);
  assert.ok(
    asserts.some((a) => /\bsurge\b/iu.test(a) && /specular/iu.test(a)),
    'no criterion-21 check names the surge beside the user-triggered specular sweep',
  );
  assert.ok(
    asserts.some((a) => /scheduled frames?/iu.test(a)),
    'criterion 21 must still ban a schedule, not a response',
  );
});

test('58, owed by p3-33: an open page draws three dots, not two', () => {
  const was = checkOf(frozenEntry('58'), 'AC-58-surfaces').assert;
  const now = checkOf(criterion('58'), 'AC-58-surfaces').assert;
  assert.match(was, /exactly one dot/u);
  assert.match(now, /\bTHREE\b/u);
});

test('66, owed by p3-35: the sort cycle is derived from the schema, and Completion is absent', () => {
  const sort = checkOf(criterion('66'), 'AC-66-sort');
  assert.equal(sort.status, 'automated');
  assert.match(sort.assert, /derived from protocol\/schema\/protocol\.json/u);
  assert.match(sort.assert, /Completion stays absent/u);
  // The derivation is the body; this is what it derives today.
  const schema = JSON.parse(
    readFileSync(fileURLToPath(new URL('../../protocol/schema/protocol.json', import.meta.url))),
  );
  const variants = schema.types.SortKey.variants;
  assert.ok(variants.includes('needs_attention'), 'SortKey carries no needs_attention');
  assert.ok(!variants.includes('completion'), 'SortKey offers completion, which is §31’s to move');
});

for (const [id, owner] of [
  ['45a', 'p3-31'],
  ['45b', 'p3-31'],
  ['45c', 'p3-31'],
  ['65', 'p3-31'],
  ['67', 'nobody: A1 leaves the negative rule intact'],
  ['64', 'nobody: acceptance_shelf.rs is phase-1 debt'],
  ['P2-25-5', 'p3-32'],
]) {
  test(`${id}, scoped not superseded (${owner}): byte-identical to the frozen copy`, () => {
    assert.deepEqual(criterion(id), frozenEntry(id));
  });
}

test('22, owed by p3-36: only the GPU clause moved, and the frozen copy proves which half', () => {
  const was = frozenEntry('22');
  const now = criterion('22');
  assert.deepEqual(checkOf(now, 'AC-22-database'), checkOf(was, 'AC-22-database'));
  const { assert: artWas, ...artRestWas } = checkOf(was, 'AC-22-art-cache');
  const { assert: artNow, ...artRestNow } = checkOf(now, 'AC-22-art-cache');
  assert.deepEqual(artRestNow, artRestWas, 'the art-cache budget and measurement moved');
  const gpu = /The GPU texture clause[^.]*\./u;
  assert.equal(artNow.replace(gpu, ''), artWas.replace(gpu, ''), 'more than the GPU clause moved');
  assert.deepEqual(
    now.checks.map((k) => k.id),
    [...was.checks.map((k) => k.id), 'AC-22-gpu-residency'],
  );
});

// ---------------------------------------------------------------------------------------------
// §36.4 and R136 — the gates that break, or silently stop testing, when a phase-3 body lands.
// Every assertion reads a SHAPE, never a line number, and every walker reads through
// `read-scanned.mjs`, prints its count and fails at zero.
// ---------------------------------------------------------------------------------------------

/** Every file under `dir` with one of `exts`, recursively, as repo-relative paths. */
function walk(dir, exts) {
  const out = [];
  for (const entry of readdirSync(join(repoRoot, dir), { withFileTypes: true })) {
    const rel = `${dir}/${entry.name}`;
    if (entry.isDirectory()) out.push(...walk(rel, exts));
    else if (exts.some((e) => entry.name.endsWith(e))) out.push(rel);
  }
  return out;
}

/** The files a walk found, read, with every vanished one skipped BEFORE it is counted. */
function readAll(files) {
  const read = [];
  for (const rel of files) {
    const text = readScannedFile(join(repoRoot, rel));
    if (text !== null) read.push({ rel, text });
  }
  return read;
}

const source = (rel) => readFileSync(join(repoRoot, rel), 'utf8');

/** One Rust function's text, from its `fn` line to the next top-level item. */
function rustFn(rel, fn) {
  const text = source(rel);
  const start = text.search(new RegExp(`^fn ${fn}\\(`, 'mu'));
  if (start === -1) return null;
  const rest = text.slice(start + 1);
  const end = rest.search(/^(?:#\[|\/\/\/|fn |pub |struct |impl |const |mod )/mu);
  return end === -1 ? rest : rest.slice(0, end);
}

test('row 1, owed by p3-34: every health_delta insert writes a layer DecayLayer declares', () => {
  const schema = JSON.parse(source('protocol/schema/protocol.json'));
  const layers = schema.types.DecayLayer.variants;
  const files = readAll([...walk('core/src', ['.rs']), ...walk('core/tests', ['.rs'])]);
  const insert = /INSERT INTO health_delta\s*\(([^)]*)\)\s*VALUES\s*\(([^)]*)\)/gu;
  const bad = [];
  let inserts = 0;
  let refusals = 0;
  for (const { rel, text } of files) {
    for (const m of text.matchAll(insert)) {
      inserts += 1;
      const columns = m[1].split(',').map((c) => c.trim());
      const value = m[2].split(',').map((v) => v.trim())[columns.indexOf('layer')] ?? '';
      const literal = /^'([^']*)'$/u.exec(value);
      if (literal === null || layers.includes(literal[1])) continue;
      // An insert asserted to FAIL is the CHECK being proved, which is the right shape: the
      // defect is a fixture that relies on the row landing. The statement ends at its `;`.
      const statement = text.slice(m.index, text.indexOf(';', m.index));
      if (/\.is_err\(\)|\.expect_err\(|\.unwrap_err\(\)/u.test(statement)) refusals += 1;
      else bad.push(`${rel}: '${literal[1]}'`);
    }
  }
  console.error(
    `health_delta inserts: ${String(inserts)} over ${String(files.length)} files, ` +
      `${String(refusals)} asserted refused`,
  );
  assert.ok(files.length > 0 && inserts > 0, 'the scan read no health_delta insert');
  assert.deepEqual(bad, [], `layers the widened CHECK rejects: ${bad.join('; ')}`);
});

test('row 2, owed by p3-34: no test claims health_delta has no producer', () => {
  assert.equal(
    rustFn('core/tests/index_schema.rs', 'health_delta_exists_with_no_producer'),
    null,
    'health_delta_exists_with_no_producer is a name that stopped being true',
  );
});

test('row 3, owed by p3-35: the top-bar floor names no widest sort; it measures every one', () => {
  const spec = source('app/e2e/topbar-floor.spec.ts');
  assert.ok(spec.length > 0);
  assert.doesNotMatch(
    spec,
    /^\s*const WIDEST_SORT\b/mu,
    'the widest label is pinned, not measured',
  );
  assert.match(spec, /widestVariant/u);
});

// §36.2 rule 7. A literal pinned on each side stays green while one implementation and its own
// literal move together, so the pair must read one shared value — the order corpus both
// comparators already read (AC-P3-35-3) — and neither side may keep a literal of its own.
test('row 4, owed by p3-35: the cursor pair reads one shared value, not a literal on each side', () => {
  const rust = rustFn(
    'core/tests/projects_list.rs',
    'the_order_key_is_the_same_cursor_the_renderer_computes',
  );
  assert.ok(rust !== null, 'the Rust half of the cursor pair is gone');
  const ts = source('app/src/renderer/shelf/page.test.ts');
  const pinned = [];
  if (/order_key_of\([^)]*\),\s*"[0-9a-f]{8}"/u.test(rust)) {
    pinned.push('core/tests/projects_list.rs');
  }
  if (/orderKeyOf\([^)]*\)\)\.toBe\('[0-9a-f]{8}'\)/u.test(ts)) {
    pinned.push('app/src/renderer/shelf/page.test.ts');
  }
  assert.deepEqual(pinned, [], `each side pins its own literal: ${pinned.join(', ')}`);
  assert.match(rust, /protocol\/shelf\/order-corpus\.json/u, 'the Rust half reads no corpus');
  assert.match(rust, /expectedOrderKey/u);
  assert.match(ts, /protocol\/shelf\/order-corpus\.json\?raw/u, 'the TS half reads no corpus');
  assert.match(ts, /expectedOrderKey/u);
});

test('row 5, owed by p3-29: has:license, has:tests and has:ci gained their producer', () => {
  const execute = source('core/src/query/execute.rs').replace(/\s+/gu, ' ');
  assert.ok(execute.length > 0);
  assert.doesNotMatch(
    execute,
    /HasAttribute::License \| HasAttribute::Tests \| HasAttribute::Ci => TermTruth::Unknown/u,
    'the three has: terms are still routed to Unknown as one arm',
  );
  assert.doesNotMatch(
    execute,
    /attribute: HasAttribute::License \| HasAttribute::Tests \| HasAttribute::Ci/u,
    'answerable() still refuses the three has: terms',
  );
});

// §36.2 rule 9: the count is deleted, not incremented. "Six" was the defect and "eight" was the
// same defect one correction later, so no written count of the jobs may stand in the prose.
test('row 6, owed by p3-29: jobs/mod.rs states no job count in its prose', () => {
  const jobs = source('core/src/jobs/mod.rs');
  assert.ok(jobs.length > 0);
  assert.doesNotMatch(jobs, /§4\.1's six jobs/u);
  assert.doesNotMatch(
    jobs,
    /\b(?:one|two|three|four|five|six|seven|eight|nine|ten|\d+)\s+(?:observation\s+)?jobs\b/iu,
    'a job count written in prose is a defect with a delay',
  );
  assert.match(jobs, /JobKind::ALL/u);
});

test('row 7, owed by p3-29: the unknown-slug fixture is not j7', () => {
  const state = source('core/tests/jobs_state.rs');
  assert.ok(state.length > 0);
  assert.doesNotMatch(state, /'j7'/u, "'j7' is a real job now, so the unknown-slug assert inverts");
});

test('row 8, owed by p3-29: the column-coverage test derives its slugs from JobKind::ALL', () => {
  const body = rustFn(
    'core/tests/jobs_state.rs',
    'every_slug_the_column_permits_is_a_job_kind_this_build_knows',
  );
  assert.ok(body !== null, 'the column-coverage test is gone');
  assert.match(body, /JobKind::ALL/u);
  assert.doesNotMatch(body, /len\(\),\s*\d|\b\d+\s*,\s*[a-z_.]*len\(\)/u, 'it pins a length');
});

test('rows 9 to 11, owed by p3-29: no test asserts a job-kind count against a literal', () => {
  const files = readAll(walk('core/tests', ['.rs']));
  const pinned = [];
  const shapes = [
    /assert_eq!\(\s*(?:JobKind::ALL|jobs|slugs)[\w.]*\.len\(\)\s*,\s*\d+\s*[,)]/u,
    /assert_eq!\(\s*\d+\s*,\s*(?:JobKind::ALL|jobs|slugs)[\w.]*\.len\(\)\s*[,)]/u,
  ];
  for (const { rel, text } of files) {
    if (shapes.some((s) => s.test(text))) pinned.push(rel);
  }
  console.error(`job-kind count pins: scanned ${String(files.length)} core test files`);
  assert.ok(files.length > 0, 'the scan read no test file');
  assert.deepEqual(pinned, []);
});

test('R136: no registered assert pins the job vocabulary at seven', () => {
  const pinned = loadRegistry(registryPath)
    .criteria.flatMap((c) => c.checks)
    .filter((k) => /seven members|holds seven\b/iu.test(String(k.assert)))
    .map((k) => k.id);
  assert.deepEqual(pinned, []);
  const kinds = checkOf(criterion('P2-21-10'), 'AC-P2-21-10-kinds');
  assert.equal(kinds.test, 'sync_phase1_untouched::the_job_vocabulary_holds_no_sync_task');
  assert.match(kinds.assert, /Derives both vocabularies/u);
});

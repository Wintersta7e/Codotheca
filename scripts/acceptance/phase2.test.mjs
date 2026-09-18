/**
 * §26.3's supersession audit and §26.5's demotion audit.
 *
 * **Every row here is applied by another plan.** §26.3 schedules each supersession into the plan
 * that lands its section's body, and a supersession scheduled and then not done is invisible —
 * the phase-1 criterion keeps passing, exactly as §26.3 warns criterion 64 would. This file is
 * the only thing that reads the schedule back against the tree.
 *
 * **Each assertion is symmetric where the row records a replacement.** An absence-only assertion
 * passes against a deletion: a plan that removed the two never-emitted tests and wrote nothing in
 * their place would satisfy "they are gone". So row 64 asserts an absence *and* a presence, and
 * so does every other row that records a replacement rather than an equality.
 *
 * **Names are compared as shapes, not as strings.** Both replaced tests are named in the doc
 * comment of the test that replaced them — which is the tree saying what it did, and is the
 * right thing for it to say. A grep for the bare name finds that prose and reports the old test
 * as still present.
 */
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { readScannedFile } from '../lib/read-scanned.mjs';
import { withoutComments } from '../lib/without-comments.mjs';
import { loadRegistry } from './registry.mjs';

const repoRoot = fileURLToPath(new URL('../..', import.meta.url));
const at = (relative) => fileURLToPath(new URL(`../../${relative}`, import.meta.url));
const readJson = (relative) => JSON.parse(readFileSync(at(relative), 'utf8'));

const registry = () => loadRegistry(at('acceptance/criteria.json'));
const frozen = () => readJson('acceptance/phase1-frozen.json');

const criterion = (reg, id) => reg.criteria.find((c) => c.id === id);
const check = (reg, criterionId, checkId) =>
  criterion(reg, criterionId)?.checks.find((c) => c.id === checkId);

/**
 * The file's text with comments blanked, and its byte count.
 *
 * Read through `readScannedFile`, which skips a vanished file rather than throwing: other gates
 * plant probes under these directories while vitest runs in parallel, and an unguarded read takes
 * this suite down instead of reporting. A file that is genuinely missing fails the assertion
 * below rather than passing as an absence.
 */
function code(relative) {
  const text = readScannedFile(at(relative));
  assert.ok(text !== null && text.length > 0, `${relative} could not be read`);
  return { bytes: text.length, stripped: withoutComments(text) };
}

// ---------------------------------------------------------------------------------------------
// §26.3 — the six rows, verified and not assumed
// ---------------------------------------------------------------------------------------------

test('§26.3/58: the band grep is narrowed to two words and the criterion is not retired', () => {
  const rule = (readJson('acceptance/forbidden.json').rules ?? []).find(
    (r) => r.id === 'c58-design-band-names',
  );
  assert.ok(rule, 'c58-design-band-names is still a rule');
  // A4: `warm` and `cooling` stay banned; the bare `\bblueprint\b` pattern is gone, because the
  // blueprint tile is an art state and not a condition band.
  assert.deepEqual(rule.patterns, ['\\bwarm\\b', '\\bcooling\\b']);
  // Narrowed, not retired — the criterion still carries all three of its checks.
  const reg = registry();
  assert.equal(criterion(reg, '58').checks.length, 3);
  assert.deepEqual(
    criterion(reg, '58').checks.map((c) => c.id),
    ['AC-58-surfaces', 'AC-58-edges', 'AC-58-vocabulary'],
  );
});

test('§26.3/64: the two never-emitted tests are gone and their replacements ran', () => {
  const rust = code('core/tests/projects_list.rs');
  const renderer = code('app/src/renderer/shelf/eras.test.ts');

  // Declarations, not mentions. Both files name the replaced test in the comment that explains
  // what replaced it; a bare-name grep reports that prose as the test still being there.
  assert.ok(
    !/\bfn\s+era_notcloned_is_never_emitted_in_phase_one\s*\(/u.test(rust.stripped),
    'core/tests/projects_list.rs still declares the never-emitted test',
  );
  assert.ok(
    !/\bit\(\s*['"`]never emits era:notcloned in phase 1/u.test(renderer.stripped),
    'eras.test.ts still declares the never-emitted test',
  );

  // The presence half. An absence-only assertion passes against a plan that deleted the two and
  // wrote nothing in their place.
  const reg = registry();
  const nine = criterion(reg, 'P2-23-9');
  assert.ok(nine, 'AC-P2-23-9 registers the replacements');
  // Compared as declarations on both sides. A cargo key is `<binary>::<fn>`; a vitest key is the
  // describe text followed by the test text, so the `it(…)` literal is a **suffix** of it and a
  // prefix match would compare the describe block instead.
  const names = nine.checks.map((c) => String(c.test));
  const rustFns = [...rust.stripped.matchAll(/^\s*(?:async\s+)?fn\s+([a-z0-9_]+)\s*\(/gmu)].map(
    (m) => m[1],
  );
  const itTexts = [...renderer.stripped.matchAll(/\bit\(\s*['"`]([^'"`]+)['"`]/gu)].map(
    (m) => m[1],
  );
  assert.ok(rustFns.length > 0 && itTexts.length > 0, 'neither replacement file declares a test');
  assert.ok(
    names.some((n) => rustFns.includes(n.split('::')[1] ?? '')),
    'no AC-P2-23-9 check names a test projects_list.rs declares',
  );
  assert.ok(
    names.some((n) => itTexts.some((t) => n.endsWith(t))),
    'no AC-P2-23-9 check names a test eras.test.ts declares',
  );

  // eslint-disable-next-line no-console -- a gate that cannot say what it compared proves nothing.
  console.error(
    `§26.3/64: projects_list.rs ${String(rust.bytes)} bytes, eras.test.ts ${String(renderer.bytes)} bytes`,
  );
});

test('§26.3/10: Peek keeps its half word for word and the page half is replaced', () => {
  const reg = registry();
  const record = frozen();
  const was = record.criteria.find((c) => c.id === '10').checks.find((c) => c.id === 'AC-10-peek');
  assert.deepEqual(check(reg, '10', 'AC-10-peek'), was);
  // §25.5 replaces the plain-text rule for the page with a sandboxed frame and a hostile
  // document, and the register names that replacement rather than dropping the clause.
  const replacement = String(check(reg, '42', 'AC-42-page')?.test ?? '');
  assert.match(replacement, /ac_p2_25_17_/u);
  assert.ok(
    reg.criteria.some((c) => c.checks.some((k) => k.test === replacement && /^P2-/u.test(c.id))),
    'no phase-2 check names the page-half replacement',
  );
});

test('§26.3/42: AC-42-peek is byte-identical and AC-42-page is superseded by §25', () => {
  const reg = registry();
  const record = frozen().criteria.find((c) => c.id === '42');
  assert.deepEqual(
    check(reg, '42', 'AC-42-peek'),
    record.checks.find((c) => c.id === 'AC-42-peek'),
  );
  const page = check(reg, '42', 'AC-42-page');
  const before = record.checks.find((c) => c.id === 'AC-42-page');
  assert.notDeepEqual(page, before, 'AC-42-page was scheduled to move and did not');
  assert.equal(page.owner, 'p2-25b');
  assert.equal(page.status, 'automated');
});

test('§26.3/45c: scoped means the register does not change', () => {
  const reg = registry();
  assert.deepEqual(
    criterion(reg, '45c'),
    frozen().criteria.find((c) => c.id === '45c'),
  );
});

test('§26.3/2 and 44: referenced and unchanged, including every test id', () => {
  const reg = registry();
  const record = frozen();
  for (const id of ['2', '44']) {
    const before = record.criteria.find((c) => c.id === id);
    // §24.10: the `test` field is the join key with no name table, so a rename reads as *not
    // run*. Byte-identity is asserted over the whole entry, which includes every one of them.
    assert.deepEqual(criterion(reg, id), before, `criterion ${id} was referenced, not amended`);
  }
});

// ---------------------------------------------------------------------------------------------
// §26.5 — the two demotions, and the attribute phase 2 does not add
// ---------------------------------------------------------------------------------------------

test('§26.5: phase 2 revives neither demoted check', () => {
  const reg = registry();
  for (const record of frozen().checks) {
    const live = check(reg, record.criterion, record.check.id);
    assert.deepEqual(live, record.check, `${record.check.id} was revived`);
    // A promotion to `automated` would be a claim of coverage nothing landed.
    assert.equal(live.status, 'deferred', `${record.check.id} claims coverage it does not have`);
  }
});

test('§26.5: data-observed-at appears nowhere in the renderer, over a non-empty walk', async () => {
  const { readdirSync } = await import('node:fs');
  const { join } = await import('node:path');
  const root = join(repoRoot, 'app/src');

  const walk = (dir) => {
    const out = [];
    let items;
    try {
      items = readdirSync(dir, { withFileTypes: true });
    } catch {
      // A root that is not there yields no files, so the count guard below is what reports it.
      // Throwing here would report ENOENT, which reads as a crash and not as "scanned nothing".
      return out;
    }
    for (const item of items) {
      const full = join(dir, item.name);
      if (item.isDirectory()) {
        // `generated` is gitignored, and a gate that greps an ignored path is the exact defect
        // the acceptance register was written against.
        if (item.name !== 'generated') out.push(...walk(full));
        continue;
      }
      if (!/\.(tsx?|css)$/u.test(item.name)) continue;
      const text = readScannedFile(full);
      // Skipped BEFORE it is counted: counting a vanished file would make the guard below stop
      // meaning what it says.
      if (text === null) continue;
      out.push({ path: full.slice(root.length + 1), text });
    }
    return out;
  };

  const files = walk(root);
  const hits = files.filter((f) => f.text.includes('data-observed-at')).map((f) => f.path);

  // eslint-disable-next-line no-console -- an absence over an empty walk is the way this dies.
  console.error(
    `§26.5: ${String(files.length)} renderer files walked, ${String(hits.length)} hits`,
  );
  assert.ok(files.length > 100, 'the walk read nothing — an absence over an empty walk is vacuous');
  assert.deepEqual(hits, [], '§25 renders an age as text under §8.4.1, not as a machine attribute');
});

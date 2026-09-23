// §34.8's standing checker, proved against a tree built to break it — and `AC-50-zero-animation`'s
// literal, proved gone from every assert rather than from the one sentence that held it.
import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkMotionClamp, KNOWN_ESCAPES, reducedClampMs } from './check-motion-clamp.mjs';
import { compareSpecificity, specificity } from './lib/motion-clamp.mjs';
import { readScannedFile } from './lib/read-scanned.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const script = join(root, 'scripts/check-motion-clamp.mjs');
const TIER = 'app/src/renderer/motion/tier.ts';

function run(args) {
  return spawnSync(process.execPath, [script, ...args], { encoding: 'utf8' });
}

/**
 * The renderer's stylesheets and `tier.ts`, copied into a scratch tree laid out like this one, so
 * a fixture can add one rule and leave everything else exactly as the product has it.
 */
function fixtureTree() {
  const dir = mkdtempSync(join(tmpdir(), 'motion-clamp-'));
  const walk = (from) => {
    for (const entry of readdirSync(from, { withFileTypes: true })) {
      const full = join(from, entry.name);
      if (entry.isDirectory()) walk(full);
      else if (entry.name.endsWith('.css')) {
        // A parallel test's probe may vanish between the walk and the read.
        const text = readScannedFile(full);
        if (text === null) continue;
        const to = join(dir, relative(root, full));
        mkdirSync(dirname(to), { recursive: true });
        writeFileSync(to, text);
      }
    }
  };
  walk(join(root, 'app/src/renderer'));
  mkdirSync(join(dir, dirname(TIER)), { recursive: true });
  writeFileSync(join(dir, TIER), readFileSync(join(root, TIER), 'utf8'));
  return dir;
}

function addSheet(dir, name, css) {
  writeFileSync(join(dir, 'app/src/renderer/styles', name), css);
}

test('ac_p3_34_14 passes over the real tree, printing what it scanned and what it matched', () => {
  const result = run([]);
  assert.equal(result.status, 0, result.stderr);
  const counts = /scanned (\d+) animated selectors, matched (\d+)/u.exec(result.stderr);
  assert.ok(counts, `the checker printed no scanned/matched count:\n${result.stderr}`);
  const scanned = Number(counts[1]);
  const matched = Number(counts[2]);
  const set = /clamp set \((\d+)\)/u.exec(result.stderr);
  assert.ok(set, 'the checker printed no derived clamp set');
  console.error(`AC-P3-34-14 real tree: scanned ${scanned}, matched ${matched}, set ${set[1]}`);
  assert.ok(scanned > 0, 'a run that scanned nothing proved nothing');
  assert.ok(matched > 0);
  assert.ok(Number(set[1]) > 0);

  const weighed = /weighed (\d+) moving selectors .* held (\d+)/u.exec(result.stderr);
  assert.ok(weighed, `the checker printed no specificity count:\n${result.stderr}`);
  console.error(`AC-P3-34-14 real tree: weighed ${weighed[1]}, held ${weighed[2]}`);
  assert.ok(Number(weighed[1]) > 0, 'a specificity pass that weighed nothing proved nothing');
  assert.equal(weighed[2], weighed[1]);
});

test('weighs a selector as a browser does', () => {
  assert.deepEqual(specificity(".cdt-card[data-hovered='true'] .cdt-dot"), [0, 3, 0]);
  assert.deepEqual(specificity("[data-effects-tier='reduced'] .cdt-dot"), [0, 2, 0]);
  assert.deepEqual(specificity("[data-effects-tier='off'] *"), [0, 1, 0]);
  assert.deepEqual(specificity('.cdt-fr-turn-show-me:hover'), [0, 2, 0]);
  assert.deepEqual(specificity('.cdt-fr-rescan-line::before'), [0, 1, 1]);
  assert.deepEqual(specificity('#root'), [1, 0, 0]);
  assert.deepEqual(specificity('body .cdt-card'), [0, 1, 1]);
  assert.ok(compareSpecificity([0, 3, 0], [0, 2, 0]) > 0);
  assert.ok(compareSpecificity([0, 2, 5], [0, 3, 0]) < 0);
  assert.equal(compareSpecificity([0, 2, 0], [0, 2, 0]), 0);
});

test('ac_p3_34_14 names a hover rule that outranks its clamp, weighing each selector alone', () => {
  const dir = fixtureTree();
  try {
    // The shape jsdom resolves wrongly: the clamp shares a comma list with a heavier `:active`
    // selector. A browser weighs `[reduced] .cdt-probe` at (0,2,0), and the hover rule wins.
    addSheet(
      dir,
      'probe.css',
      '.cdt-probe { transform: scale(1); }\n' +
        ".cdt-card[data-hovered='true'] .cdt-probe { transform: scale(1.5); }\n" +
        "[data-effects-tier='reduced'] .cdt-card:active,\n" +
        "[data-effects-tier='reduced'] .cdt-probe,\n" +
        "[data-effects-tier='off'] .cdt-probe {\n  transform: none;\n}\n",
    );
    const outranked = checkMotionClamp(dir).failures.filter((f) => f.includes('cdt-probe'));
    console.error(`AC-P3-34-14 outranked clamp:\n${outranked.join('\n')}`);
    assert.equal(outranked.length, 1, outranked.join('\n'));
    assert.ok(
      outranked[0]?.includes("`.cdt-card[data-hovered='true'] .cdt-probe` outranks its clamp"),
    );
    assert.ok(outranked[0]?.includes('transform at reduced: (0,3,0) against (0,2,0)'));
    assert.ok(outranked[0]?.includes('transform at off: (0,3,0) against (0,2,0)'));

    // Naming the hovered state in the clamp is what makes it win.
    addSheet(
      dir,
      'probe-fixed.css',
      "[data-effects-tier='reduced'] .cdt-card[data-hovered='true'] .cdt-probe,\n" +
        "[data-effects-tier='off'] .cdt-card[data-hovered='true'] .cdt-probe {\n" +
        '  transform: none;\n}\n',
    );
    assert.deepEqual(
      checkMotionClamp(dir).failures.filter((f) => f.includes('cdt-probe')),
      [],
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('ac_p3_34_14 names a new unclamped animated class and exits non-zero', () => {
  const dir = fixtureTree();
  try {
    // The copy is faithful: with nothing added it passes exactly as the real tree does.
    assert.equal(run(['--root', dir]).status, 0);

    addSheet(
      dir,
      'rogue.css',
      '.cdt-rogue { animation: rogue-sweep 900ms linear both; }\n' +
        '.cdt-rogue-fade { transition: opacity 900ms linear; }\n',
    );
    const result = run(['--root', dir]);
    console.error(`AC-P3-34-14 with a rogue class:\n${result.stderr}`);
    assert.notEqual(result.status, 0, 'an unclamped animated class passed the gate');
    const failures = result.stderr
      .split('\n')
      .filter((line) => line.includes('is unclamped') || line.includes('KNOWN_ESCAPES'));
    assert.ok(failures.some((line) => line.includes('`.cdt-rogue` is unclamped at reduced')));
    assert.ok(
      failures.some((line) => line.includes('`.cdt-rogue-fade` is unclamped at reduced and off')),
    );
    // Nothing else failed: the known escapes did not mask the new one and did not add to it.
    assert.ok(
      failures.every((line) => line.includes('cdt-rogue')),
      failures.join('\n'),
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('ac_p3_34_14 reads the clamp from tier.ts rather than restating it', () => {
  const real = reducedClampMs(readFileSync(join(root, TIER), 'utf8'));
  assert.ok(real !== null && real > 0, 'tier.ts declares no REDUCED_CLAMP_MS');

  const dir = fixtureTree();
  try {
    // Longer than today's clamp, shorter than the one written below.
    const slow = real + 100;
    addSheet(dir, 'slow.css', `.cdt-slow { transition: opacity ${String(slow)}ms linear; }\n`);
    const before = checkMotionClamp(dir).failures.filter((f) => f.includes('cdt-slow'));
    assert.ok(before.length > 0, `a ${String(slow)}ms transition passed a ${String(real)}ms clamp`);

    const tier = join(dir, TIER);
    writeFileSync(
      tier,
      readFileSync(tier, 'utf8').replace(
        /REDUCED_CLAMP_MS\s*=\s*\d+/u,
        `REDUCED_CLAMP_MS = ${String(slow + 100)}`,
      ),
    );
    const after = checkMotionClamp(dir);
    console.error(`AC-P3-34-14 clamp moved to ${String(after.clampMs)}ms`);
    assert.equal(after.clampMs, slow + 100);
    assert.deepEqual(
      after.failures.filter((f) => f.includes('cdt-slow')),
      [],
      'the threshold did not move with the constant',
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('ac_p3_34_14 fails a run that scans nothing', () => {
  const dir = mkdtempSync(join(tmpdir(), 'motion-clamp-empty-'));
  try {
    const result = run(['--root', dir]);
    console.error(`AC-P3-34-14 empty tree:\n${result.stderr}`);
    assert.notEqual(result.status, 0, 'a gate that scanned nothing passed');
    assert.match(result.stderr, /scanned 0 animated selectors/u);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('ac_p3_34_14 lets the known-escape list only shrink', () => {
  // The real list is empty — every escape it landed with is clamped, and a name is never added
  // to pass the gate — so the mechanism is proved over a fixture entry instead.
  assert.equal(KNOWN_ESCAPES.size, 0, `KNOWN_ESCAPES grew: ${[...KNOWN_ESCAPES.keys()].join(' ')}`);
  const name = 'cdt-listed-escape';
  const known = new Map([[name, 'a fixture entry, unclamped at reduced']]);
  const dir = fixtureTree();
  try {
    addSheet(dir, 'listed.css', `.${name} { animation: listed-sweep 900ms linear both; }\n`);
    // Listed and still escaping: excused, and nothing else fails.
    const excused = checkMotionClamp(dir, known);
    assert.deepEqual(excused.known, [name]);
    assert.deepEqual(excused.failures, []);

    // Clamp it at both tiers: it stops escaping, so its entry must go.
    addSheet(
      dir,
      'fixed.css',
      `[data-effects-tier='reduced'] .${name},\n[data-effects-tier='off'] .${name} {\n` +
        '  animation: none;\n  transition: none;\n}\n',
    );
    const result = checkMotionClamp(dir, known);
    assert.ok(
      result.failures.some((f) => f.includes('KNOWN_ESCAPES') && f.includes(`.${name}`)),
      `a fixed escape stayed on the list silently: ${result.failures.join('\n')}`,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

const NUMBER =
  '(?:\\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|' +
  'fifteen|sixteen|seventeen|eighteen|nineteen|twenty)';
const COUNT_BY_CLASS_NAMES = new RegExp(
  `\\b${NUMBER}\\s+(?:[\\w-]+\\s+){0,2}class[- ]names?\\b`,
  'iu',
);

/**
 * A sentence that talks about the clamp and puts a number against *class names* — the shape of
 * the literal, not one spelling of it, so a reworded restatement is caught too.
 */
export function statesClampCount(text) {
  return text
    .split(/(?<=[.;:])\s+/u)
    .some((sentence) => /clamp/iu.test(sentence) && COUNT_BY_CLASS_NAMES.test(sentence));
}

function asserts(node, out = []) {
  if (Array.isArray(node)) for (const item of node) asserts(item, out);
  else if (node !== null && typeof node === 'object') {
    if (typeof node.assert === 'string') out.push({ id: node.id, text: node.assert });
    for (const value of Object.values(node)) asserts(value, out);
  }
  return out;
}

test('ac_p3_34_15 no acceptance assert states the clamp class-name count', () => {
  // The detector first, on the sentence the register used to carry and on a rewording of it.
  assert.ok(statesClampCount('The clamp selects ten class names exactly.'));
  assert.ok(statesClampCount('Exactly 12 distinct class names sit in the tier clamp.'));
  assert.ok(!statesClampCount('The clamp is derived; the checker prints the set and its size.'));

  const registry = JSON.parse(readFileSync(join(root, 'acceptance/criteria.json'), 'utf8'));
  const all = asserts(registry);
  const offending = all.filter((a) => statesClampCount(a.text)).map((a) => a.id);
  console.error(`AC-P3-34-15 asserts scanned: ${String(all.length)}`);
  assert.ok(all.length > 0, 'a scan over no assert proved nothing');
  assert.deepEqual(offending, [], 'an assert states the clamp count a checker derives');
});

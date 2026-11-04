/**
 * [p4] §39's reach: Ship, Tend and the rhythm stay inside the window. A module that reads them
 * names no notification, tray, window-title, taskbar or app-icon API — the escape list imported
 * from the health escape gate, its one owner, never a copy.
 *
 * Every file is read through `scripts/lib/read-scanned.mjs`, so one that vanished between the
 * walk and the read is skipped before it is counted. Comments are stripped first, so a sentence
 * naming a rule never trips it. Test files are not product and are not scanned.
 */
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from 'vitest';

import { collectFiles, ESCAPE_CALLS, SCAN_ROOTS } from '../../scripts/check-health-escape.mjs';
import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';
import { withoutComments } from '../../scripts/lib/without-comments.mjs';

const REPO = fileURLToPath(new URL('../..', import.meta.url));

/** A reader names one of the three types, or reads one of the three fields. */
const READER = /\b(?:ShipRing|TendRing|RhythmState)\b|\.(?:ship|tend|rhythm)\b/u;

interface Reach {
  readonly scanned: number;
  readonly readers: readonly string[];
  /** `path: call` for each escape call a reader names. */
  readonly hits: readonly string[];
}

/** Every product source under `roots` of `base`: which read the rings, and what they escape by. */
function reach(roots: readonly string[], base: string): Reach {
  let scanned = 0;
  const readers: string[] = [];
  const hits: string[] = [];
  for (const file of collectFiles(roots, base)) {
    const path = relative(base, file).split(sep).join('/');
    if (/\.test\.tsx?$/u.test(path) || path.startsWith('app/src/generated/')) continue;
    const text = readScannedFile(file);
    if (text === null) continue;
    scanned += 1;
    const code = withoutComments(text);
    if (!READER.test(code)) continue;
    readers.push(path);
    for (const call of ESCAPE_CALLS) if (code.includes(call)) hits.push(`${path}: ${call}`);
  }
  return { scanned, readers, hits };
}

// [p4] §39.12: no ship, tend or rhythm figure leaves the window.
test('AC-P4-39-12 no ship, tend or rhythm reader names an escape API', () => {
  const found = reach(SCAN_ROOTS, REPO);
  console.warn(
    `AC-P4-39-12: ${String(found.scanned)} file(s) scanned, ` +
      `${String(found.readers.length)} reader(s): ${found.readers.join(', ')}; ` +
      `${String(ESCAPE_CALLS.length)} escape call(s) checked`,
  );
  expect(found.scanned, 'a scan that read nothing proves nothing').toBeGreaterThan(0);
  expect(
    found.readers.length,
    'the band reads the rhythm, so zero readers is a blind scan',
  ).toBeGreaterThan(0);
  expect(ESCAPE_CALLS.length).toBeGreaterThan(0);
  expect(found.hits).toEqual([]);
});

test('the reach scan names a reader planted beside an escape call', () => {
  const dir = mkdtempSync(join(tmpdir(), 'rings-reach-'));
  try {
    mkdirSync(join(dir, 'src'));
    writeFileSync(
      join(dir, 'src', 'planted.ts'),
      'export function tell(state: { rhythm: { activeDays: number } }): void {\n' +
        '  const days = state.rhythm.activeDays;\n' +
        "  new Notification('days', { body: String(days) });\n" +
        '}\n',
    );
    writeFileSync(join(dir, 'src', 'quiet.ts'), "new Notification('no ring here');\n");
    const found = reach(['src'], dir);
    expect(found.scanned).toBe(2);
    expect(found.readers).toEqual(['src/planted.ts']);
    expect(found.hits).toEqual(['src/planted.ts: new Notification(']);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

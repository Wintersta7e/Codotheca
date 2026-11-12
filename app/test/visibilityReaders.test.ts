/**
 * §56.8 (AC-P4-56-23): shelf, palette and count membership read one function, `isOnShelf`
 * (`app/src/renderer/shelf/visibility.ts`). Every renderer read of `.isHidden` or `.isExcluded`
 * outside it is listed here by its own text with the reason it is not a membership test, so a
 * new `!row.isHidden` filter fails this test instead of quietly letting excluded rows through.
 */
import { readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from 'vitest';

import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';

const REPO = fileURLToPath(new URL('../..', import.meta.url));
const RENDERER = join(REPO, 'app/src/renderer');
const READ = /\.is(?:Hidden|Excluded)\b/u;

/** `[file under app/src/renderer, a piece of the line, why it is not a membership test]`. */
const CLASSIFIED: readonly (readonly [string, string, string])[] = [
  ['shelf/visibility.ts', 'return !row.isHidden && !row.isExcluded;', 'the function itself'],
  ['shelf/evaluate.ts', 'return row.isHidden;', "`is:hidden`'s own term"],
  ['shelf/evaluate.ts', 'return row.isExcluded;', "`is:excluded`'s own term"],
  [
    'shelf/evaluate.ts',
    "if (row.isHidden && !asksFor('hidden')) return false;",
    "§8.0b's per-flag opt-in, which a single predicate cannot express",
  ],
  [
    'shelf/evaluate.ts',
    "if (row.isExcluded && !asksFor('excluded')) return false;",
    "§8.0b's per-flag opt-in, which a single predicate cannot express",
  ],
  ['app/useLibrary.ts', 'isHidden: data.isHidden === true,', 'parses the flags_changed event'],
];

function sourcesUnder(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== 'generated') out.push(...sourcesUnder(full));
    } else if (/\.tsx?$/u.test(entry.name) && !/\.test\.tsx?$/u.test(entry.name)) out.push(full);
  }
  return out;
}

test('AC-P4-56-23: every renderer reader of isHidden/isExcluded is isOnShelf or classified', () => {
  let scanned = 0;
  let readers = 0;
  const used = new Set<number>();
  const unclassified: string[] = [];
  for (const file of sourcesUnder(RENDERER)) {
    const text = readScannedFile(file);
    if (text === null) continue;
    scanned += 1;
    const rel = relative(RENDERER, file).replaceAll('\\', '/');
    text.split('\n').forEach((line, i) => {
      if (!READ.test(line)) return;
      readers += 1;
      const hit = CLASSIFIED.findIndex(([f, piece]) => f === rel && line.includes(piece));
      if (hit === -1) unclassified.push(`${rel}:${String(i + 1)}: ${line.trim()}`);
      else used.add(hit);
    });
  }
  console.error(
    `AC-P4-56-23: visibility readers: ${String(readers)} reads in ${String(scanned)} files`,
  );
  expect(scanned, 'the walk read no renderer source').toBeGreaterThan(0);
  expect(readers, 'the walk found no isHidden/isExcluded read').toBeGreaterThan(0);
  expect(unclassified, 'call isOnShelf, or classify the read with a reason').toEqual([]);
  const stale = CLASSIFIED.filter((_, i) => !used.has(i)).map(([f, piece]) => `${f}: ${piece}`);
  expect(stale, 'classified entries that match no line').toEqual([]);
});

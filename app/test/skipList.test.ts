import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test, expect } from 'vitest';
import { EXCLUSION_LIST, EXCLUSION_CAPTION } from '../src/shared/skipList';
import { required } from '../src/shared/required';

const REPO_ROOT = fileURLToPath(new URL('../..', import.meta.url));

/** The literals of the core's SKIP_LIST, in source order. */
function coreSkipList(): string[] {
  const source = readFileSync(join(REPO_ROOT, 'core/src/scan/skiplist.rs'), 'utf8');
  const start = source.indexOf('pub const SKIP_LIST');
  expect(start, 'SKIP_LIST is not where this test expects it').not.toBe(-1);
  const open = source.indexOf('[', source.indexOf('=', start));
  const close = source.indexOf('];', open);
  const body = source.slice(open + 1, close);
  return [...body.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((m) =>
    required(m[1], 'skip-list entry').replace(/\\(.)/g, '$1'),
  );
}

// §10.1b: the rendered strings are §4.3's, `$RECYCLE.BIN` included, because a privacy policy
// that misspells what it matches is false.
test('the drawn exclusion list is the core exclusion list, character for character', () => {
  const fromCore = coreSkipList();
  // A gate whose passing run reads zero entries is a failing gate.
  expect(fromCore.length).toBeGreaterThan(0);
  expect([...EXCLUSION_LIST]).toEqual(fromCore);
});

// §10.1b: all 30 entries, in §4.3's order — caches, then build outputs, then system paths, and
// last §24.3b's staging directory — never alphabetised, so it can be checked against a real
// machine.
test('the list is all 30 entries and is not alphabetised', () => {
  expect(EXCLUSION_LIST.length).toBe(30);
  const sorted = [...EXCLUSION_LIST].sort();
  expect([...EXCLUSION_LIST]).not.toEqual(sorted);
});

test('the caption names the list as the policy', () => {
  expect(EXCLUSION_CAPTION).toBe('This list is the privacy policy. Editable in settings.');
});

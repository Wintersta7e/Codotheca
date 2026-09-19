import { test, expect } from 'vitest';
import * as copy from './copy';
import { SKIP_AHEAD_LABEL } from '../a11y/names'; // R12: plan 12b declares it.
import { LOCKFILE_NAMES } from '../../shared/lockfileNames';

/** Every rendered string this module exports, however deeply it is nested. */
function everyString(value: unknown): string[] {
  if (typeof value === 'string') return [value];
  if (Array.isArray(value)) return value.flatMap(everyString);
  if (typeof value === 'object' && value !== null) return Object.values(value).flatMap(everyString);
  return [];
}

// §10.1: this is the claim the screen invites the user to check against their own disk. v1
// claimed "opens .git directories only" and then specified README, manifest and image reads.
test('the consent paragraph is what phase 1 actually reads', () => {
  expect(copy.CONSENT_PARAGRAPH).toContain('names and timestamps');
  expect(copy.CONSENT_PARAGRAPH).toContain('README, LICENSE, and package manifests');
  expect(copy.CONSENT_PARAGRAPH).toContain('256 KB each');
  // [p3] §32.6: **the paragraph names every lock file the read opens and how deep it looks.**
  // A11.3's *"rides the existing grant"* is true only after this sentence moves — the shipped
  // text promised a named set *at the repository root*, and a lock file is neither a manifest nor
  // root-only, so the grant as written did not cover the read that now ships.
  //
  // The names are read from the shared list rather than restated, so the sentence and the reader
  // cannot drift.
  for (const name of LOCKFILE_NAMES) {
    expect(copy.CONSENT_PARAGRAPH, name).toContain(name);
  }
  expect(LOCKFILE_NAMES.length, 'a loop over no names proves nothing').toBe(6);
  expect(copy.CONSENT_PARAGRAPH).toContain('three directories deep');
  expect(copy.CONSENT_PARAGRAPH).toContain('16 MB each');
  // **The root-only claim is gone**, because the set it described is no longer the whole of what
  // is read.
  expect(copy.CONSENT_PARAGRAPH).not.toContain('a small named set of files at the repository root');
  // Unchanged, and still true: a lock file is machine-generated bookkeeping, not source text.
  expect(copy.CONSENT_PARAGRAPH).toContain('does not read the text of your source files');
  // [p3] §29.8: the claim is now qualified, because J7 reads the text once the user grants it.
  // An unqualified *never* would be false for exactly the user who said yes.
  expect(copy.CONSENT_PARAGRAPH).toContain('does not read the text of your source files unless');
  expect(copy.CONSENT_PARAGRAPH).toContain('off until you do');
  expect(copy.CONSENT_PARAGRAPH).toContain('Nothing is uploaded');
  expect(copy.CONSENT_PARAGRAPH).toContain('no account');
});

// §10.1b: row 1's prototype note is `READ FROM .git · NEEDED NOW`, word for word the claim
// §10.1 exists to kill.
test('consent row 1 names the root files as well as .git', () => {
  const [first] = copy.CONSENT_ROWS;
  expect(first?.kind).toBe('control');
  expect(first?.note).toBe('READ FROM .git AND FOUR ROOT FILES · NEEDED NOW');
});

// §10.1b: rows 2 and 3 are statements, because a checkbox there would store a preference
// nothing reads.
test('two of the three consent rows are statements and not controls', () => {
  expect(copy.CONSENT_ROWS).toHaveLength(3);
  expect(copy.CONSENT_ROWS.filter((r) => r.kind === 'control')).toHaveLength(1);
  expect(copy.CONSENT_ROWS[2]?.body).toContain('commit-days, releases and revivals');
});

// §10.1a: this sentence does more trust work than any privacy paragraph.
test('the root list is captioned with the two files it read', () => {
  expect(copy.ROOTS_LIST_CAPTION).toBe("from your editor's recent projects and .gitconfig");
});

// §10.1b: the prototype's "You can use the app while it works." is false by the design's own
// performance rule — every full-screen flow unmounts the shelf.
test('the DIG note claims only what is true', () => {
  expect(copy.DIG_NOTE).toBe('The scan keeps running after you leave this screen.');
  expect(copy.DIG_INERT_NOTE).toBe('NOTHING TO INDEX WITHOUT THIS.');
});

// §10.3a: `Enough — show me what you've got` is superseded wherever it appears, §10.4
// included, on typographic grounds: a 32-character sentence cannot be set in a 26px box at
// .14em.
// R12: the label is plan 12b's, so it is asserted where it is declared. What this file still
// owns is the negative: the superseded sentence must appear in none of its strings.
test('the escape control is the short one', () => {
  expect(SKIP_AHEAD_LABEL).toBe('SKIP AHEAD');
  expect(copy).not.toHaveProperty('SKIP_AHEAD_LABEL');
  const all = everyString(copy);
  expect(all.some((s) => s.includes("show me what you've got"))).toBe(false);
});

// §10.4a: span is the one figure monotone under increasing coverage, so AT LEAST is exactly
// honest and needs no chip.
test('the headline takes AT LEAST below full coverage and nothing above it', () => {
  expect(copy.revealHeadline(12, true)).toBe('YOU HAVE BEEN AT THIS FOR 12 YEARS');
  expect(copy.revealHeadline(12, false)).toBe('YOU HAVE BEEN AT THIS FOR AT LEAST 12 YEARS');
  expect(copy.revealHeadline(1, true)).toBe('YOU HAVE BEEN AT THIS FOR 1 YEAR');
  expect(copy.revealHeadline(null, true)).toBe('HISTORY IS STILL ARRIVING');
});

// §10.4a: historyComplete needs its own string because the coverage string implies growth,
// while a history still arriving can move BEST YEAR to a different year.
test('the two coverage strings say different things', () => {
  expect(copy.COVERAGE_PARTIAL(212)).toBe('across the 212 projects indexed so far');
  expect(copy.COVERAGE_HISTORY_GROWS).toBe('history still indexing — this can only grow');
  expect(copy.COVERAGE_HISTORY_MOVES).toBe('history still indexing — this can move, not just grow');
});

// §17 and the Global Constraints: the token FORGET appears in no rendered string.
test('no rendered string carries a destructive verb, and the word setup appears nowhere', () => {
  const all = everyString(copy);
  // A gate whose passing run scans zero strings is a failing gate.
  expect(all.length).toBeGreaterThan(20);
  for (const line of all) {
    expect(line).not.toMatch(/\bFORGET\b/i);
    expect(line).not.toMatch(/\bsetup\b/i);
    expect(line).not.toMatch(/\b(delete|uninstall|checkout)\b/i);
  }
});

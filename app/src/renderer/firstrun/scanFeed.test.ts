import { expect, test } from 'vitest';
import {
  INITIAL_SCAN_FEED,
  MILESTONES,
  TALLY_CAP,
  milestoneCrossed,
  scanFeedReducer,
  scanLineText,
} from './scanFeed';
import type { ScanFeedEvent, ScanFeedState } from './scanFeed';
import type { ProjectId } from '../../generated/protocol';

const pid = (n: number): ProjectId => n as ProjectId;

const upsert = (id: number, name: string, lang: string | null): ScanFeedEvent => ({
  kind: 'upserted',
  id: pid(id),
  name,
  primaryLanguage: lang,
});
const progress = (indexed: number, dirs = 0, repos = 0): ScanFeedEvent => ({
  kind: 'progress',
  indexedProjects: indexed,
  walkedDirs: dirs,
  foundRepos: repos,
});
const fold = (events: readonly ScanFeedEvent[], from = INITIAL_SCAN_FEED): ScanFeedState =>
  events.reduce(scanFeedReducer, from);

// §10.2: a count, never a percentage. The denominator is unknown and the walk discovers as it
// goes, so nothing here may name a total or hold a value shaped like a ratio.
test('the feed exposes no total and no fraction', () => {
  const state = fold([upsert(1, 'alpha', 'Rust'), progress(1, 900, 1), { kind: 'flush' }]);
  for (const [key, value] of Object.entries(state)) {
    expect(key).not.toMatch(/total|percent|ratio|fraction|denominator/i);
    // A completion ratio's shape: a non-integer strictly inside 0..1.
    if (typeof value === 'number') {
      expect(Number.isInteger(value) || value < 0 || value > 1, key).toBe(true);
    }
  }
});

// §10.3: batches on a fixed cadence, never one per discovery.
test('a discovery waits for the flush and then never moves again', () => {
  const held = fold([upsert(1, 'alpha', null), upsert(2, 'beta', null)]);
  expect(held.tiles).toHaveLength(0);
  expect(held.pending).toHaveLength(2);
  const flushed = scanFeedReducer(held, { kind: 'flush' });
  expect(flushed.tiles.map((t) => t.name)).toEqual(['alpha', 'beta']);
  expect(flushed.pending).toHaveLength(0);
  // §10.3's ban is on re-sorting: a later arrival appends and disturbs nothing.
  const later = fold([upsert(3, 'gamma', null), { kind: 'flush' }], flushed);
  expect(later.tiles.map((t) => t.name)).toEqual(['alpha', 'beta', 'gamma']);
});

// §10.3a: unlit is absent-grey, never missing. The tile lands with no language and takes one
// when J1 resolves — in place, without leaving its position.
test('the second wave powers a tile on without moving it', () => {
  const state = fold([
    upsert(1, 'alpha', null),
    upsert(2, 'beta', null),
    { kind: 'flush' },
    upsert(1, 'alpha', 'Rust'),
    { kind: 'flush' },
  ]);
  expect(state.tiles).toHaveLength(2);
  expect(state.tiles[0]?.primaryLanguage).toBe('Rust');
  expect(state.tiles[0]?.name).toBe('alpha');
});

// §10.3a requirement 2: a merge decrements the headline in the same frame the fade starts.
// Otherwise the headline counts discoveries and the scan ends on a different number than the
// reveal — the disagreement §10.4 exists to prevent.
test('a merge decrements the headline immediately and fades exactly one tile', () => {
  const before = fold([
    upsert(1, 'alpha', 'Rust'),
    upsert(2, 'alpha-dup', 'Rust'),
    { kind: 'flush' },
    progress(2),
  ]);
  expect(before.found).toBe(2);
  const merged = scanFeedReducer(before, { kind: 'merged', from: pid(2), into: pid(1) });
  expect(merged.found).toBe(1);
  expect(merged.merging.has(pid(2))).toBe(true);
  expect(merged.tiles).toHaveLength(2);
  // The next flush completes the 200 ms removal; the count does not move a second time.
  const gone = scanFeedReducer(merged, { kind: 'flush' });
  expect(gone.tiles.map((t) => t.id)).toEqual([pid(1)]);
  expect(gone.found).toBe(1);
});

// The core's own indexedProjects already reflects the merge, so a fresh progress frame must not
// subtract it twice.
test('a progress frame after a merge does not double-count it', () => {
  const state = fold([
    upsert(1, 'alpha', null),
    upsert(2, 'beta', null),
    { kind: 'flush' },
    progress(2),
    { kind: 'merged', from: pid(2), into: pid(1) },
    progress(1),
  ]);
  expect(state.found).toBe(1);
});

// §10.3a: sorted by count descending, at most 7. A running fact, never a progress readout.
//
// The fixture uses ten languages that carry ten *distinct* sigils. `languageCode` maps everything
// outside its table to one fallback code, so a fixture of assorted rare languages collapses into
// a single chip — which is correct behaviour (two chips reading the same two letters with
// different counts would be unreadable) and would silently test nothing about the cap.
test('the tally is capped and ordered by count descending', () => {
  const langs = [
    'Rust',
    'Rust',
    'Rust',
    'TypeScript',
    'TypeScript',
    'Python',
    'C++',
    'C#',
    'JavaScript',
    'Java',
    'Go',
    'Shell',
    'Lua',
  ];
  const state = fold([
    ...langs.map((l, i) => upsert(i + 1, `p${String(i)}`, l)),
    { kind: 'flush' },
  ]);
  expect(new Set(state.tally.map((c) => c.sigil)).size).toBe(state.tally.length);
  expect(state.tally).toHaveLength(TALLY_CAP);
  expect(state.tally[0]?.count).toBe(3);
  const counts = state.tally.map((c) => c.count);
  expect([...counts].sort((a, b) => b - a)).toEqual(counts);
});

// A project with no classified language contributes to no chip rather than to an UNKNOWN one —
// "never render unknown as zero", applied to a tally.
test('the tally never invents a language for an unclassified project', () => {
  const state = fold([
    upsert(1, 'alpha', 'Rust'),
    upsert(2, 'beta', 'Go'),
    upsert(3, 'gamma', null),
    upsert(4, 'delta', null),
    { kind: 'flush' },
  ]);
  expect(state.tiles).toHaveLength(4);
  expect(state.tally.reduce((n, c) => n + c.count, 0)).toBe(2);
});

// §10.2 and §10.3a: 10 / 50 / 100 / 250 / 500, each crossed once.
test('a milestone fires once on the crossing and never on a plateau', () => {
  expect(MILESTONES).toEqual([10, 50, 100, 250, 500]);
  expect(milestoneCrossed(9, 10)).toBe(10);
  expect(milestoneCrossed(10, 11)).toBeNull();
  expect(milestoneCrossed(9, 12)).toBe(10);
  // A batch that jumps two milestones reports the higher one — a preview of what is indexed at
  // that instant, not a queue of celebrations.
  expect(milestoneCrossed(48, 101)).toBe(100);
  // A merge that drops the count below a milestone does not re-fire it on the way back up.
  expect(milestoneCrossed(11, 10)).toBeNull();
});

// §10.3a: those counts live on the line and never in the 64px headline, which stays the project
// count. Directories walked is a progress number for a denominator that does not exist.
test('the scan line reads directories and repositories, and thousands are grouped', () => {
  expect(scanLineText(214903, 147)).toBe('214,903 directories walked · 147 repositories');
  expect(scanLineText(0, 0)).toBe('0 directories walked · 0 repositories');
});

test('finishing sets the settle flag and stops nothing else', () => {
  const state = fold([upsert(1, 'alpha', null), { kind: 'flush' }, { kind: 'finished' }]);
  expect(state.finished).toBe(true);
  expect(state.tiles).toHaveLength(1);
});

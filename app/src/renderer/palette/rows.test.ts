import { describe, expect, it } from 'vitest';
import type { LocationId, ProjectId } from '../../generated/protocol.js';
import { makeLocationRef, makeProjectRow } from '../testing/projectRow.js';
import {
  PALETTE_ROW_CAP,
  PALETTE_UNAVAILABLE_TEXT,
  isPaletteCandidate,
  paletteCountText,
  paletteMatches,
  paletteRowAction,
  selectPaletteRows,
} from './rows.js';

describe('isPaletteCandidate', () => {
  it('is §8.6’s predicate and nothing else', () => {
    expect(isPaletteCandidate(makeProjectRow())).toBe(true);
    expect(isPaletteCandidate(makeProjectRow({ isReference: true }))).toBe(false);
    expect(isPaletteCandidate(makeProjectRow({ isHidden: true }))).toBe(false);
    expect(isPaletteCandidate(makeProjectRow({ isSubmodule: true }))).toBe(false);
    // Archived and pinned are not in the predicate; a launcher that hid archived work would
    // be a fourth filter the spec does not carry.
    expect(isPaletteCandidate(makeProjectRow({ isArchived: true }))).toBe(true);
    expect(isPaletteCandidate(makeProjectRow({ isPinned: true }))).toBe(true);
  });
});

describe('paletteMatches', () => {
  const row = makeProjectRow({ name: 'Nightfall', primaryLanguage: 'Rust' });

  it('matches on name and primary_language, case-insensitively', () => {
    expect(paletteMatches(row, 'night')).toBe(true);
    expect(paletteMatches(row, 'FALL')).toBe(true);
    expect(paletteMatches(row, 'rus')).toBe(true);
    expect(paletteMatches(row, 'zzz')).toBe(false);
  });

  it('matches everything on an empty query, and ignores surrounding space', () => {
    expect(paletteMatches(row, '')).toBe(true);
    expect(paletteMatches(row, '   ')).toBe(true);
  });

  it('does not match on description or owner — those are §8.3’s bare terms, not this', () => {
    const described = makeProjectRow({ name: 'a', owner: 'octo', description: 'a launcher' });
    expect(paletteMatches(described, 'octo')).toBe(false);
    expect(paletteMatches(described, 'launcher')).toBe(false);
  });
});

describe('selectPaletteRows', () => {
  it('orders last_touched_at descending and ties on id ascending', () => {
    const all = [
      makeProjectRow({ id: 3 as ProjectId, name: 'c', lastTouchedAt: 100 }),
      makeProjectRow({ id: 1 as ProjectId, name: 'a', lastTouchedAt: 300 }),
      makeProjectRow({ id: 2 as ProjectId, name: 'b', lastTouchedAt: 300 }),
    ];
    expect(selectPaletteRows(all, '').rows.map((r) => r.name)).toEqual(['a', 'b', 'c']);
  });

  it('caps the rendered list at 40 while the count states the full match set', () => {
    const all = Array.from({ length: 57 }, (_, i) =>
      makeProjectRow({
        id: (i + 1) as ProjectId,
        name: `hit-${String(i)}`,
        lastTouchedAt: 1000 - i,
      }),
    );
    const sel = selectPaletteRows(all, 'hit');
    expect(sel.rows).toHaveLength(PALETTE_ROW_CAP);
    expect(sel.matched).toBe(57);
    expect(sel.total).toBe(57);
  });

  it('counts candidates in the denominator, never the whole library', () => {
    const all = [
      makeProjectRow({ id: 1 as ProjectId, name: 'keep' }),
      makeProjectRow({ id: 2 as ProjectId, name: 'keep two' }),
      makeProjectRow({ id: 3 as ProjectId, name: 'ref', isReference: true }),
      makeProjectRow({ id: 4 as ProjectId, name: 'hidden', isHidden: true }),
    ];
    const sel = selectPaletteRows(all, 'keep two');
    expect(sel.matched).toBe(1);
    expect(sel.total).toBe(2);
    expect(paletteCountText(sel)).toBe('1 OF 2');
  });
});

describe('paletteRowAction', () => {
  it('launches against the primary location', () => {
    const row = makeProjectRow({ primaryLocation: makeLocationRef(7) });
    expect(paletteRowAction(row)).toEqual({ kind: 'launch', locationId: 7 as LocationId });
  });

  // Never claim currency you do not have: with no primary location there is nothing to open,
  // and the row says so rather than offering an inert LAUNCH.
  it('is unavailable when no location is primary', () => {
    expect(paletteRowAction(makeProjectRow({ primaryLocation: null }))).toEqual({
      kind: 'unavailable',
    });
    expect(PALETTE_UNAVAILABLE_TEXT).toBe('NO COPY ON THIS MACHINE');
  });
});

import { describe, expect, it } from 'vitest';
import type { LocationId, ProjectId } from '../../generated/protocol.js';
import { makeLocationRef, makeProjectRow, notClonedRow } from '../testing/projectRow.js';
import {
  PALETTE_INSTALL_TEXT,
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

  // §24.5's leading key. The tail-section rule expressed in the one dimension a flat list has:
  // whatever §23 gives a zero-location project for `last_touched_at` must not defeat it, which
  // is why the fixture gives the not-cloned row the largest value in the set.
  it('sorts every not-cloned row after every row that has a location', () => {
    const all = [
      notClonedRow({ id: 1 as ProjectId, name: 'blueprint', lastTouchedAt: 9_999 }),
      makeProjectRow({ id: 2 as ProjectId, name: 'cold', lastTouchedAt: 100 }),
      notClonedRow({ id: 3 as ProjectId, name: 'blueprint two', lastTouchedAt: 9_998 }),
      makeProjectRow({ id: 4 as ProjectId, name: 'warm', lastTouchedAt: 300 }),
    ];
    expect(selectPaletteRows(all, '').rows.map((r) => r.name)).toEqual([
      'warm',
      'cold',
      'blueprint',
      'blueprint two',
    ]);
  });

  // The cap is what the key protects: 40 blueprints must not consume a list of 41 projects.
  it('keeps not-cloned rows from consuming the 40-row cap', () => {
    const all = [
      ...Array.from({ length: 40 }, (_, i) =>
        notClonedRow({
          id: (i + 1) as ProjectId,
          name: `hit-blueprint-${String(i)}`,
          lastTouchedAt: 9_000 + i,
        }),
      ),
      makeProjectRow({ id: 99 as ProjectId, name: 'hit-located', lastTouchedAt: 1 }),
    ];
    const sel = selectPaletteRows(all, 'hit');
    expect(sel.rows[0]?.name).toBe('hit-located');
    expect(sel.matched).toBe(41);
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

  // §24.5: a project with no `location` row at all. The palette does not clone — it offers the
  // page where the destination is chosen, and the ellipsis is that promise.
  it('offers INSTALL for a project with no location row', () => {
    expect(paletteRowAction(notClonedRow({ id: 4 as ProjectId }))).toEqual({
      kind: 'install',
      projectId: 4 as ProjectId,
    });
    expect(PALETTE_INSTALL_TEXT).toBe('↵ INSTALL…');
  });

  // Never claim currency you do not have: a copy that is offline or gone cannot be opened, and
  // the row states the reason rather than offering an inert LAUNCH. §24.5 keeps this case its
  // own words — it is a different fact from having no copy at all.
  it('is unavailable when every copy is offline or missing', () => {
    for (const presence of ['offline', 'missing'] as const) {
      expect(
        paletteRowAction(makeProjectRow({ primaryLocation: makeLocationRef(7), presence })),
        presence,
      ).toEqual({ kind: 'unavailable' });
    }
    expect(PALETTE_UNAVAILABLE_TEXT).toBe('NO COPY ON THIS MACHINE');
  });

  // `unscanned` is a location nobody has looked at, not a location that is gone. Claiming
  // NO COPY ON THIS MACHINE about it would be the false-absence defect wearing the fix's clothes.
  it('leaves an unscanned copy launchable rather than claiming it is absent', () => {
    expect(
      paletteRowAction(
        makeProjectRow({ primaryLocation: makeLocationRef(9), presence: 'unscanned' }),
      ),
    ).toEqual({ kind: 'launch', locationId: 9 as LocationId });
  });
});

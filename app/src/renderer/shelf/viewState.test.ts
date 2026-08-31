import { describe, expect, it } from 'vitest';
import type { ProjectId, ViewState } from '../../generated/protocol.js';
import { densityStep } from '../card/geometry.js';
import {
  DEFAULT_DENSITY_PX,
  DEFAULT_SHELF_VIEW,
  DENSITY_TILE_PX,
  SORT_KEYS,
  SORT_LABELS,
  clampDensity,
  nextDensity,
  nextSort,
  patchFor,
  viewFromState,
} from './viewState.js';

const state = (over: Partial<ViewState> = {}): ViewState => ({
  query: '',
  sort: 'last_touched',
  viewMode: 'grid',
  density: 186,
  collapsedSections: [],
  scrollOffset: 0,
  selectedProjectId: null,
  dismissedNotices: [],
  windowGeometry: null,
  savedAt: null,
  ...over,
});

describe('the density ladder', () => {
  it("is exactly three steps and they are §8.0a's entire domain", () => {
    expect(DENSITY_TILE_PX).toEqual([148, 186, 232]);
    expect(DEFAULT_DENSITY_PX).toBe(186);
  });
  it("puts one step in each of §7.7's three rendering bands", () => {
    const names = DENSITY_TILE_PX.map((px) => densityStep(px).name);
    expect(names).toEqual(['compact', 'default', 'roomy']);
  });
  it('cycles, and wraps', () => {
    expect(nextDensity(148)).toBe(186);
    expect(nextDensity(186)).toBe(232);
    expect(nextDensity(232)).toBe(148);
  });
  it('snaps a stored value that is not a step to the nearest one', () => {
    // 240 is §11.3a's superseded value; a stored row from a build that shipped it must not
    // render an off-ladder tile.
    expect(clampDensity(240)).toBe(232);
    expect(clampDensity(150)).toBe(148);
  });
  it('falls back to the default rather than to zero', () => {
    // The CSS fallback in var(--tile,186px) is the same rule: a missing or unreadable row
    // renders the default step, never an unstyled grid and never a zero-width tile.
    expect(clampDensity(null)).toBe(DEFAULT_DENSITY_PX);
    expect(clampDensity(undefined)).toBe(DEFAULT_DENSITY_PX);
    expect(clampDensity(0)).toBe(DEFAULT_DENSITY_PX);
    expect(clampDensity(Number.NaN)).toBe(DEFAULT_DENSITY_PX);
    expect(clampDensity(Number.POSITIVE_INFINITY)).toBe(DEFAULT_DENSITY_PX);
  });
  it('cycles from an off-ladder stored value without ever leaving the ladder', () => {
    // nextDensity(240) must be a step, not 241: the control is the only writer of the column
    // and a value it cannot reach again would strand the ladder.
    expect(DENSITY_TILE_PX).toContain(nextDensity(240));
    expect(nextDensity(240)).toBe(148);
  });
});

describe('the sort ladder', () => {
  it('cycles exactly three keys', () => {
    expect(SORT_KEYS).toEqual(['last_touched', 'name', 'size']);
    expect(nextSort('last_touched')).toBe('name');
    expect(nextSort('name')).toBe('size');
    expect(nextSort('size')).toBe('last_touched');
  });
  it('drops Completion — a key over an uncomputed column orders by unknown', () => {
    expect(Object.keys(SORT_LABELS)).not.toContain('completion');
    expect(Object.values(SORT_LABELS).join(' ')).not.toMatch(/completion/i);
  });
  it('labels them as §8.0a spells them', () => {
    expect(SORT_LABELS.last_touched).toBe('Last touched');
    expect(SORT_LABELS.name).toBe('Name');
    expect(SORT_LABELS.size).toBe('Size');
  });
  it('labels every key it cycles, so the control can never render undefined', () => {
    for (const key of SORT_KEYS) expect(SORT_LABELS[key].length).toBeGreaterThan(0);
    expect(Object.keys(SORT_LABELS)).toHaveLength(SORT_KEYS.length);
  });
});

describe('viewFromState', () => {
  it('parses the query once, so the pills and the filter cannot disagree', () => {
    const view = viewFromState(state({ query: 'is:dirty' }));
    expect(view.ast.terms).toEqual([{ kind: 'flag', negated: false, flag: 'dirty' }]);
  });
  it('turns the stored section ids into collapse state', () => {
    const view = viewFromState(state({ collapsedSections: ['era:2019'] }));
    expect(view.collapsed.get('era:2019')).toBe(true);
  });
  it('clamps a stored density that is off the ladder', () => {
    expect(viewFromState(state({ density: 240 })).density).toBe(232);
  });
  it('restores every field §1.9 persists', () => {
    const view = viewFromState(
      state({
        viewMode: 'list',
        scrollOffset: 940,
        selectedProjectId: 12 as ProjectId,
        dismissedNotices: ['notice.dismissed.identityAsk'],
      }),
    );
    expect(view.viewMode).toBe('list');
    expect(view.scrollOffset).toBe(940);
    expect(view.selectedProjectId).toBe(12);
    expect(view.dismissedNotices).toEqual(['notice.dismissed.identityAsk']);
  });
});

describe('patchFor', () => {
  it('writes nothing when nothing changed', () => {
    expect(patchFor(DEFAULT_SHELF_VIEW, DEFAULT_SHELF_VIEW)).toBeNull();
  });
  it('sends only the changed field; null means leave unchanged', () => {
    const next = { ...DEFAULT_SHELF_VIEW, sort: 'name' as const };
    const patch = patchFor(DEFAULT_SHELF_VIEW, next);
    expect(patch?.sort).toBe('name');
    expect(patch?.query).toBeNull();
    expect(patch?.viewMode).toBeNull();
  });
  it('never writes a scroll offset alongside a density change', () => {
    // §8.0a: a density change re-anchors on the focused project id, never the pixel offset —
    // the offset addresses a different row once the grid re-cuts. The offset must be dropped
    // even when the caller moved it in the same step, which is exactly when it is wrong.
    const next = { ...DEFAULT_SHELF_VIEW, density: 232, scrollOffset: 1200 };
    const patch = patchFor(DEFAULT_SHELF_VIEW, next);
    expect(patch?.density).toBe(232);
    expect(patch?.scrollOffset).toBeNull();
  });
  it('writes a scroll offset when the density did not move', () => {
    // The mirror of the case above: without it, dropping the offset unconditionally would pass.
    const next = { ...DEFAULT_SHELF_VIEW, scrollOffset: 1200 };
    expect(patchFor(DEFAULT_SHELF_VIEW, next)?.scrollOffset).toBe(1200);
  });
  it('serialises collapse state back to stable section ids', () => {
    const next = { ...DEFAULT_SHELF_VIEW, collapsed: new Map([['era:tail', true]]) };
    expect(patchFor(DEFAULT_SHELF_VIEW, next)?.collapsedSections).toEqual(['era:tail']);
  });
  it('never writes the window geometry, which is the shell’s to own', () => {
    const next = { ...DEFAULT_SHELF_VIEW, sort: 'name' as const };
    expect(patchFor(DEFAULT_SHELF_VIEW, next)?.windowGeometry).toBeNull();
  });
});

import { describe, expect, it } from 'vitest';

import type { DebtItem, DebtScoring, DebtItemState, DecayLayer } from '../../generated/protocol';
import { groupByLayer, scoredOpenIn } from './groupByLayer';

function item(
  layer: DecayLayer,
  state: DebtItemState = 'open',
  scoring: DebtScoring = 'scored',
): DebtItem {
  return {
    source: 'todo_marker',
    fingerprint: `${layer}-${state}-${scoring}`,
    state,
    scoring,
    layer,
    pathDisplay: 'a.rs',
    line: 1,
    column: 1,
    salientText: 'TODO: a thing',
    firstSeenAt: 1,
    lastSeenAt: 1,
    basis: 'head',
  };
}

describe('groupByLayer', () => {
  it('leaves a layer with no items ABSENT, never an empty array', () => {
    const grouped = groupByLayer([item('dust'), item('rust')]);

    expect([...grouped.keys()].sort()).toEqual(['dust', 'rust']);
    expect(grouped.has('cobwebs')).toBe(false);
    expect(grouped.get('cobwebs')).toBeUndefined();
    // The distinction this shape exists for: `[]` and absent would be two spellings of one fact.
    expect(grouped.get('dust')).toHaveLength(1);
  });

  it('is empty for an empty list rather than five empty lanes', () => {
    expect(groupByLayer([]).size).toBe(0);
  });

  it('carries an unverified item through and never counts it', () => {
    const items = [item('dust'), item('dust', 'unverified')];
    const grouped = groupByLayer(items);

    expect(grouped.get('dust')).toHaveLength(2);
    expect(grouped.get('dust')?.some((i) => i.state === 'unverified')).toBe(true);
    expect(scoredOpenIn(items)).toBe(1);
  });

  it('carries a shown_only item through and never counts it', () => {
    const items = [item('rust'), item('rust', 'open', 'shown_only')];

    expect(groupByLayer(items).get('rust')).toHaveLength(2);
    expect(scoredOpenIn(items)).toBe(1);
  });

  it('counts an empty list as 0, which is a measured zero and not an unknown', () => {
    expect(scoredOpenIn([])).toBe(0);
  });

  it('counts nothing when every item is unverified or shown_only', () => {
    expect(scoredOpenIn([item('dust', 'unverified'), item('rust', 'open', 'shown_only')])).toBe(0);
  });

  it('keeps every item of one layer in the order it arrived', () => {
    const a = item('overgrowth');
    const b = { ...item('overgrowth'), fingerprint: 'second' };
    expect(groupByLayer([a, b]).get('overgrowth')).toEqual([a, b]);
  });
});

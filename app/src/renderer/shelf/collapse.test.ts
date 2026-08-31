import { describe, expect, it } from 'vitest';
import { ERA_COLLAPSE_MIN_ORDER, ERA_COLLAPSE_THRESHOLD } from './eras.js';
import { isCollapsed, parseCollapseState, serializeCollapseState, toggled } from './collapse.js';

describe('the view_state encoding', () => {
  it('round-trips explicit collapse and explicit expansion', () => {
    const state = parseCollapseState(['era:2019', '!era:2018']);
    expect(state.get('era:2019')).toBe(true);
    expect(state.get('era:2018')).toBe(false);
    expect([...serializeCollapseState(state)].sort()).toEqual(['!era:2018', 'era:2019']);
  });
  it('leaves an untouched section undefined, which is not the same as expanded', () => {
    expect(parseCollapseState([]).has('era:2019')).toBe(false);
  });
  it('survives a round trip through the wire shape it is stored as', () => {
    // `view_state.collapsedSections` is a `[String]`; a list of ids alone cannot say *the user
    // explicitly opened this one*, which is the whole reason for the `!` prefix.
    const entries = ['era:live', '!era:2019', 'era:tail'];
    const state = parseCollapseState(entries);
    expect(
      [...serializeCollapseState(parseCollapseState([...serializeCollapseState(state)]))].sort(),
    ).toEqual([...entries].sort());
  });
});

describe('isCollapsed', () => {
  const none = parseCollapseState([]);
  it('collapses nothing below the 150-item threshold', () => {
    expect(isCollapsed(none, 'era:2019', 11, ERA_COLLAPSE_THRESHOLD - 1, true)).toBe(false);
    expect(isCollapsed(none, 'era:2019', 11, ERA_COLLAPSE_THRESHOLD, true)).toBe(false);
  });
  it('collapses sections older than EARLIER THIS YEAR past the threshold', () => {
    expect(isCollapsed(none, 'era:2019', 11, ERA_COLLAPSE_THRESHOLD + 1, true)).toBe(true);
    expect(isCollapsed(none, 'era:live', 0, 151, true)).toBe(false);
    expect(isCollapsed(none, 'era:year', 3, 151, true)).toBe(false);
  });
  it('takes the boundary from the one constant, so the two cannot drift', () => {
    expect(isCollapsed(none, 'era:x', ERA_COLLAPSE_MIN_ORDER, 151, true)).toBe(true);
    expect(isCollapsed(none, 'era:x', ERA_COLLAPSE_MIN_ORDER - 1, 151, true)).toBe(false);
  });
  it('a non-empty query suppresses auto-collapse entirely', () => {
    // Otherwise a filter that matches only old work reads as changing a counter and nothing else.
    expect(isCollapsed(none, 'era:2019', 11, 151, false)).toBe(false);
  });
  it('an explicit toggle beats both the threshold and the query', () => {
    const expanded = parseCollapseState(['!era:2019']);
    expect(isCollapsed(expanded, 'era:2019', 11, 151, true)).toBe(false);
    const collapsed = parseCollapseState(['era:live']);
    expect(isCollapsed(collapsed, 'era:live', 0, 10, true)).toBe(true);
    expect(isCollapsed(collapsed, 'era:live', 0, 10, false)).toBe(true);
  });
});

describe('toggled', () => {
  it('records the explicit decision without disturbing the others', () => {
    const next = toggled(parseCollapseState(['era:2019']), 'era:2018', false);
    expect(next.get('era:2019')).toBe(true);
    expect(next.get('era:2018')).toBe(false);
  });
  it('does not mutate the state it was handed', () => {
    const before = parseCollapseState(['era:2019']);
    toggled(before, 'era:2018', true);
    expect(before.has('era:2018')).toBe(false);
  });
});

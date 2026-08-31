import { ERA_COLLAPSE_MIN_ORDER, ERA_COLLAPSE_THRESHOLD } from './eras.js';

/** `true` = explicitly collapsed, `false` = explicitly expanded, absent = never toggled. */
export type CollapseState = ReadonlyMap<string, boolean>;

const EXPANDED_PREFIX = '!';

export function parseCollapseState(entries: readonly string[]): CollapseState {
  const state = new Map<string, boolean>();
  for (const entry of entries) {
    if (entry.startsWith(EXPANDED_PREFIX)) state.set(entry.slice(1), false);
    else state.set(entry, true);
  }
  return state;
}

export function serializeCollapseState(state: CollapseState): readonly string[] {
  return [...state].map(([id, collapsed]) => (collapsed ? id : `${EXPANDED_PREFIX}${id}`));
}

/**
 * §8.1: the threshold counts **rendered** rows (Reference has its own block), a non-empty query
 * suppresses auto-collapse entirely, and an explicit toggle beats both.
 */
export function isCollapsed(
  state: CollapseState,
  sectionId: string,
  order: number,
  renderedTotal: number,
  queryIsEmpty: boolean,
): boolean {
  const explicit = state.get(sectionId);
  if (explicit !== undefined) return explicit;
  if (!queryIsEmpty) return false;
  return renderedTotal > ERA_COLLAPSE_THRESHOLD && order >= ERA_COLLAPSE_MIN_ORDER;
}

export function toggled(state: CollapseState, sectionId: string, next: boolean): CollapseState {
  const copy = new Map(state);
  copy.set(sectionId, next);
  return copy;
}

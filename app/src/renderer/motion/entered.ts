/**
 * §11.6: entry animations play once, per card and per view, and "has entered" is tracked
 * **outside render state**. Two bugs came from an animation string reverting to its mount
 * value when a transient state cleared, which re-ran entry and read as a reload.
 */
const entered = new Map<string, Set<string>>();

function bucket(viewKey: string): Set<string> {
  const existing = entered.get(viewKey);
  if (existing !== undefined) return existing;
  const created = new Set<string>();
  entered.set(viewKey, created);
  return created;
}

/** True exactly once per (view, id). Call it where the animation string is chosen. */
export function markEntered(viewKey: string, id: string): boolean {
  const set = bucket(viewKey);
  if (set.has(id)) return false;
  set.add(id);
  return true;
}

export function hasEntered(viewKey: string, id: string): boolean {
  return entered.get(viewKey)?.has(id) ?? false;
}

/**
 * Every full-screen flow unmounts the shelf (§11.6) — the view that re-enters is a new view,
 * and its cards may animate again. Pass no key to clear every view.
 */
export function resetEntered(viewKey?: string): void {
  if (viewKey === undefined) entered.clear();
  else entered.delete(viewKey);
}

/**
 * `value`, or a thrown error naming what was missing. For a test's lookup whose absence is the
 * failure — a DOM query, an index, a regex capture — so the type narrows by a check that runs
 * rather than by a `!` or `as` that only claims it. Imported by tests only; nothing in the bundle
 * reaches it.
 */
export function required<T>(value: T | null | undefined, what: string): T {
  if (value === null || value === undefined) throw new Error(`no ${what}`);
  return value;
}

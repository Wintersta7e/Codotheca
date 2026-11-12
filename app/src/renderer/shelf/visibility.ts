/**
 * §56.8: the renderer's one visibility function. A project is on the shelf, in the palette and in
 * every count when it is neither hidden nor fully excluded — the same rule the core's SQL fragment
 * (`core/src/exclusion/visibility.rs`) states, read from the two flags the wire row carries.
 * `app/test/visibilityReaders.test.ts` fails on a reader of either flag outside its allow-list.
 */
export function isOnShelf(row: {
  readonly isHidden: boolean;
  readonly isExcluded: boolean;
}): boolean {
  return !row.isHidden && !row.isExcluded;
}

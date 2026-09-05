/** §8.3's `query_grammar_version`. Adding an enum value bumps it; removing a field is forbidden. */
export const QUERY_GRAMMAR_VERSION = 1;

export const QUERY_FIELDS = [
  'lang',
  'owner',
  'in',
  'is',
  'has',
  'touched',
  'size',
  'completion',
  'collection',
] as const;
export type QueryField = (typeof QUERY_FIELDS)[number];

export const IS_FLAGS = [
  'dirty',
  'unpushed',
  'behind',
  'interrupted',
  'archived',
  'pinned',
  'hidden',
  'reference',
  'bare',
  'fork',
  'empty',
  'shallow',
  'local',
  'wsl',
  'new',
  // §23.6. `is:remote` never ships under that name: the design's `is:remote` means `remoteOnly`
  // in its own prototype — the identical predicate — so this keeps the word the shelf already
  // prints over that section. One added VALUE, and no new field: `newfield:x` is an ignored term
  // today and would become a filter, which is not backward-compatible.
  'notcloned',
] as const;
export type IsFlag = (typeof IS_FLAGS)[number];

export const HAS_ATTRIBUTES = [
  'readme',
  'license',
  'tests',
  'ci',
  'remote',
  'stash',
  'submodules',
] as const;
export type HasAttribute = (typeof HAS_ATTRIBUTES)[number];

/** Binary units. `size_tracked_bytes` is HEAD blob bytes (§5.3); the base is fixed here so the
 *  two engines cannot disagree about what `size:>10mb` means. */
export const SIZE_UNIT_BYTES = { kb: 1024, mb: 1048576, gb: 1073741824 } as const;
export type SizeUnit = keyof typeof SIZE_UNIT_BYTES;

export const TOUCHED_UNIT_DAYS = { d: 1, w: 7, mo: 30, y: 365 } as const;
export type TouchedUnit = keyof typeof TOUCHED_UNIT_DAYS;

/** §8.3a: `completion:` parses and is then dropped from the effective query. Nothing computes
 *  `completion_lit`, and both readings of a comparison against NULL are wrong. */
export const NEVER_EVALUATED = ['completion'] as const;

export function isQueryField(value: string): value is QueryField {
  return (QUERY_FIELDS as readonly string[]).includes(value);
}
export function isIsFlag(value: string): value is IsFlag {
  return (IS_FLAGS as readonly string[]).includes(value);
}
export function isHasAttribute(value: string): value is HasAttribute {
  return (HAS_ATTRIBUTES as readonly string[]).includes(value);
}

/**
 * The criterion tag. Every acceptance test carries `AC-<criterion>` in its own name, because a
 * table mapping criterion to test name is a second copy of the truth and rots on the first
 * rename. Rust identifiers cannot hold a hyphen, so `ac_14_…` is the same tag as `AC-14 …`.
 *
 * The trailing guard is what keeps `ac_12balance` from reading as criterion 12b.
 *
 * Two grammars, one per phase, and they stay two. `TAG` cannot reach past `p2`, so before
 * `TAG_P2` existed a phase-2 test name produced **zero** tags — and `joinResults` reports a
 * result as untagged only when it carries at least one, so the net that catches a renamed test
 * was blind to the entire phase. `tags.test.mjs` asserts that blindness so nobody folds the two
 * back into one.
 */
export const TAG = /\bac[-_ ]?(\d{1,2}[a-c]?)(?=$|[^0-9a-z])/giu;

/** §26.1's `AC-P2-<section>-<n>`. `2[0-5]` because §26 owns no criterion of its own. */
export const TAG_P2 = /\bac[-_ ]?p2[-_ ](2[0-5])[-_ ](\d{1,2})(?=$|[^0-9a-z])/giu;

export function tagsIn(name) {
  const text = String(name);
  const found = [];
  for (const match of text.matchAll(TAG)) {
    const tag = match[1].toLowerCase();
    if (!found.includes(tag)) found.push(tag);
  }
  for (const match of text.matchAll(TAG_P2)) {
    const tag = `P2-${match[1]}-${match[2]}`;
    if (!found.includes(tag)) found.push(tag);
  }
  return found;
}

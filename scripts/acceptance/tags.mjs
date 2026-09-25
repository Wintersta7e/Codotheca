/**
 * The criterion tag. Every acceptance test carries `AC-<criterion>` in its own name, because a
 * table mapping criterion to test name is a second copy of the truth and rots on the first
 * rename. Rust identifiers cannot hold a hyphen, so `ac_14_…` is the same tag as `AC-14 …`.
 *
 * The trailing guard is what keeps `ac_12balance` from reading as criterion 12b.
 *
 * Four grammars, one per phase, and they stay four. `TAG` cannot reach past `p2`, so before
 * `TAG_P2` existed a phase-2 test name produced **zero** tags — and `joinResults` reports a
 * result as untagged only when it carries at least one, so the net that catches a renamed test
 * was blind to the entire phase. It went blind the same way to all 146 phase-3 criteria until
 * `TAG_P3`, while the run reported `0 problems`, and `TAG_P4` exists before the first phase-4
 * test does. `tags.test.mjs` asserts each blindness so nobody folds them back into one.
 */
export const TAG = /\bac[-_ ]?(\d{1,2}[a-c]?)(?=$|[^0-9a-z])/giu;

/** §26.1's `AC-P2-<section>-<n>`. `2[0-5]` because §26 owns no criterion of its own. */
export const TAG_P2 = /\bac[-_ ]?p2[-_ ](2[0-5])[-_ ](\d{1,2})(?=$|[^0-9a-z])/giu;

/**
 * §36.1's `AC-P3-<section>-<n>`. `2[89]|3[0-5]` because §36 owns no criterion of its own, and the
 * numeric segment carries the optional letter: `AC-P3-30-11a` is a different criterion from
 * `AC-P3-30-11` and both exist. Copying `TAG_P2` verbatim would match `11` and then fail the
 * guard on `a`, which is R44's lettered-value failure repeating on the one id §36.1 names — and
 * it is invisible in 144 of the 146 cases.
 *
 * The trailing guard is unchanged in kind and is what keeps `ac_p3_30_11abc` from reading as
 * criterion `30-11a`: `[a-c]?` takes `a`, the guard rejects `b`, the backtrack takes the empty
 * letter, the guard rejects `a`, and the match fails — which is correct, because that name names
 * no criterion.
 */
export const TAG_P3 = /\bac[-_ ]?p3[-_ ](2[89]|3[0-5])[-_ ](\d{1,2}[a-c]?)(?=$|[^0-9a-z])/giu;

/**
 * §49.1's `AC-P4-<section>-<n>`. `3[89]|4[0-8]` because §37 is the scope section and §49 the
 * register contract, neither owning a criterion; no letter, because phase 4 has none.
 */
export const TAG_P4 = /\bac[-_ ]?p4[-_ ](3[89]|4[0-8])[-_ ](\d{1,2})(?=$|[^0-9a-z])/giu;

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
  for (const match of text.matchAll(TAG_P3)) {
    const tag = `P3-${match[1]}-${match[2].toLowerCase()}`;
    if (!found.includes(tag)) found.push(tag);
  }
  for (const match of text.matchAll(TAG_P4)) {
    const tag = `P4-${match[1]}-${match[2]}`;
    if (!found.includes(tag)) found.push(tag);
  }
  return found;
}

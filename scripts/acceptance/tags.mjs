/**
 * The criterion tag. Every acceptance test carries `AC-<criterion>` in its own name, because a
 * table mapping criterion to test name is a second copy of the truth and rots on the first
 * rename. Rust identifiers cannot hold a hyphen, so `ac_14_…` is the same tag as `AC-14 …`.
 *
 * The trailing guard is what keeps `ac_12balance` from reading as criterion 12b.
 */
export const TAG = /\bac[-_ ]?(\d{1,2}[a-c]?)(?=$|[^0-9a-z])/giu;

export function tagsIn(name) {
  const found = [];
  for (const match of String(name).matchAll(TAG)) {
    const tag = match[1].toLowerCase();
    if (!found.includes(tag)) found.push(tag);
  }
  return found;
}

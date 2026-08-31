/**
 * §5.3's `size_tracked_bytes`, formatted once for the whole product (**R12**).
 *
 * The shelf's era header and the project page's stat rail each carried their own formatter; the
 * second divided by 1000 and carried `B`, `KB` and `TB` rungs, so one repository printed two
 * different sizes on the shelf and on its own page. §8.1 decides between them: *one decimal of GB
 * at ≥ 0.1 GB, whole MB below*, on the design's own scale rule, which divides MB by 1024. There
 * is no rung below MB.
 *
 * The figure is returned alone. §8.1's era header appends `tracked` (§5.3's mandatory word),
 * §8.4.1's Peek fact labels it `TRACKED`, and §7.8's hover strip labels it not at all.
 */
export const BYTES_PER_MB = 1024 ** 2;
export const BYTES_PER_GB = 1024 ** 3;

/** §8.1's switch point, named so the two branches cannot drift apart. */
export const GB_FLOOR_BYTES = 0.1 * BYTES_PER_GB;

export function formatTrackedBytes(bytes: number): string {
  // One decimal of *precision*, not a forced decimal place: `41 GB`, `1.2 GB`, `0.2 GB`.
  // The prototype computes `Math.round(total / 1024 * 10) / 10` and concatenates it
  // (`Codotheca v7 Shelf.dc.html:2683`, `:2696`), and it outranks its prose files. That is the
  // only reading under which §8.1's "one decimal of GB" and its own era-header examples —
  // `41 GB tracked` at `08-shelf-query.md:31`, `:93` and `03-git.md:66` — agree.
  if (bytes >= GB_FLOOR_BYTES) {
    return `${String(Math.round((bytes / BYTES_PER_GB) * 10) / 10)} GB`;
  }
  // A measured zero is not an unknown: `0 MB` is a fact, and §7.8a draws the same distinction
  // for `is_pinned = 0`. A caller with no measurement passes nothing and renders no figure.
  return `${String(Math.round(bytes / BYTES_PER_MB))} MB`;
}

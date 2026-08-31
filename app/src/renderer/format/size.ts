/**
 * §5.3's `size_tracked_bytes`, formatted once for the whole product (**R12**).
 *
 * The shelf's era header and the project page's stat rail each carried their own formatter; the
 * second divided by 1000 and carried `B`, `KB` and `TB` rungs, so one repository printed two
 * different sizes on the shelf and on its own page. §8.1 decides between them: *one decimal of GB
 * at ≥ 0.1 GB, whole MB below*, on the design's own scale rule, which divides MB by 1024. There
 * is no rung below MB.
 *
 * *One decimal of GB* is a statement about **precision, not about padding**: the figure is
 * rounded to a tenth and printed as a plain number, so a whole one prints `41 GB` and a
 * fractional one `1.2 GB`. That is what reconciles §8.1's sentence with the era-header examples
 * printed beside it, which read `41 GB tracked`; forcing a trailing `.0` is the one reading under
 * which the two contradict each other.
 *
 * The figure is returned alone. §8.1's era header appends `tracked` (§5.3's mandatory word),
 * §8.4.1's Peek fact labels it `TRACKED`, and §7.8's hover strip labels it not at all.
 */
export const BYTES_PER_MB = 1024 ** 2;
export const BYTES_PER_GB = 1024 ** 3;

/** §8.1's switch point, named so the two branches cannot drift apart. */
export const GB_FLOOR_BYTES = 0.1 * BYTES_PER_GB;

export function formatTrackedBytes(bytes: number): string {
  if (bytes >= GB_FLOOR_BYTES) {
    return `${String(Math.round((bytes / BYTES_PER_GB) * 10) / 10)} GB`;
  }
  // A measured zero is not an unknown: `0 MB` is a fact, and §7.8a draws the same distinction
  // for `is_pinned = 0`. A caller with no measurement passes nothing and renders no figure.
  return `${String(Math.round(bytes / BYTES_PER_MB))} MB`;
}

/**
 * §1.10's playtime ledger, formatted once for the whole renderer (**R20**).
 *
 * It sits here rather than in the project page's own rail strings because §8.4.1's Peek prints
 * the same figure and the shelf is built first: two implementations would print different
 * durations for one project on Peek and on its own page — R12's failure, one ledger over.
 *
 * **The unit is the hour, and `0h` is a measured fact.** §8.4.1 states it outright: *"A fact whose
 * job has not run renders `—`, never `0`. `PLAYTIME 0h` is the one exception and is true: that
 * ledger starts at install."* This is the single place in the product where a rendered zero is a
 * measurement rather than an unknown, and it is deliberately carved out of *never render unknown
 * as zero*. Every other spec site renders hours too — `10-first-run.md:266` prints
 * `Playtime 0h — starts now`, `08-palette-tokens.md:471` prints `6h tracked, 9 commit-days`.
 *
 * **Sub-hour values carry one decimal of *precision*, not a forced decimal place** — `0h`, `0.2h`,
 * `2.1h`, `6h`, `40h`. The spec fixes only `0h` and `6h` and says nothing about what sits between
 * them; this is the one rule under which both render verbatim while a real 45-minute session stays
 * visible. Flooring to whole hours would print `0h` for it, collapsing *installed and never
 * played* together with *played most of an hour* into one string — and `0h`'s whole point is that
 * it is a fact. The rounding is the one already settled for `formatTrackedBytes`' gigabytes, so
 * the product has one rounding grammar rather than two.
 *
 * **This is the *playtime* ledger and only that.** §7.8's bench scrim renders a live session as
 * `AT THE BENCH · 2h 07m` (`07-art-composition.md:274`) and is 12c's `useBenchElapsed`: elapsed
 * wall time on an open session, never summed into playtime (§1.10 keeps the two ledgers apart).
 * Two formatters is correct here, and the different grammar is what keeps them visibly distinct —
 * folding them together would be R12 inverted.
 */
export function formatPlaytime(seconds: number): string {
  const hours = Math.max(0, seconds) / 3600;
  return `${String(Math.round(hours * 10) / 10)}h`;
}

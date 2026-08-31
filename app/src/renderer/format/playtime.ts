/**
 * §1.10's playtime ledger, formatted once for the whole renderer.
 *
 * It sits here rather than inside the project page because Peek prints the same figure: two
 * implementations would print different durations for one project on Peek and on its own page.
 *
 * **An empty ledger reads `0h`, not `0m`.** §8.1 states the rule and its reason: a fact whose job
 * has not run renders an em dash and never a zero, and `PLAYTIME 0h` is *the one exception,
 * because that ledger starts at install*. §10's reveal calls the same figure "complete by
 * construction, and the only figure on the screen that is". So this zero is a measured fact, and
 * the hour unit is what says so: `0m` reads as a rounding of something, where `0h` reads as a
 * ledger that has recorded nothing yet.
 *
 * This is the *playtime* ledger and only that. §7.8's bench figure shares the grammar **above
 * zero** and is deliberately not shared code: it is elapsed wall time on a live session, never
 * summed into playtime, and one function across both ledgers is how they get summed by accident.
 * At zero they diverge, because only this one has a ledger to declare complete.
 */
export function formatPlaytime(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds / 60));
  if (total === 0) return '0h';
  const hours = Math.floor(total / 60);
  const minutes = total % 60;
  if (hours === 0) return `${String(minutes)}m`;
  return `${String(hours)}h ${String(minutes).padStart(2, '0')}m`;
}

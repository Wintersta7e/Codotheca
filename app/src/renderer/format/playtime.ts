/**
 * §1.10's playtime ledger, formatted once for the whole renderer (**R20**).
 *
 * It sits here rather than in the project page's own rail strings because §8.4.1's Peek prints
 * the same figure and the shelf is built first: two implementations would print different
 * durations for one project on Peek and on its own page — R12's failure, one ledger over.
 *
 * This is the *playtime* ledger and only that. §7.8's bench figure shares the grammar and is
 * deliberately not shared code: it is elapsed wall time on a live session and is never summed
 * into playtime (§1.10 keeps the two ledgers apart), so the card pins that grammar separately.
 */
export function formatPlaytime(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds / 60));
  const hours = Math.floor(total / 60);
  const minutes = total % 60;
  if (hours === 0) return `${String(minutes)}m`;
  return `${String(hours)}h ${String(minutes).padStart(2, '0')}m`;
}

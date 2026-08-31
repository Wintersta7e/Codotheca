/**
 * §1.10's playtime ledger, formatted once for the whole renderer.
 *
 * It sits here rather than inside the project page because Peek prints the same figure: two
 * implementations would print different durations for one project on Peek and on its own page.
 *
 * This is the *playtime* ledger and only that. §7.8's bench figure shares the grammar and is
 * deliberately **not** shared code: it is elapsed wall time on a live session and is never summed
 * into playtime, and one function across both ledgers is how they get summed by accident. The two
 * are pinned equal by test instead.
 */
export function formatPlaytime(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds / 60));
  const hours = Math.floor(total / 60);
  const minutes = total % 60;
  if (hours === 0) return `${String(minutes)}m`;
  return `${String(hours)}h ${String(minutes).padStart(2, '0')}m`;
}

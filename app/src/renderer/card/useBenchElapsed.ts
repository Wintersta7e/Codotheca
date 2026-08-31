import { useEffect, useState } from 'react';
import { useWindowActive } from '../motion/useWindowActive';

/**
 * §7.8's live tile. The figure is **elapsed wall time since the session opened, not
 * `credited_seconds`** — segments decide credit (§9) — so it is labelled, never summed into
 * playtime, and never shown as a total. Two ledgers, never merged.
 *
 * It reconciles with criterion 21 three ways: displayed resolution is minutes, so it repaints at
 * most once per minute; it **re-derives from `started_at`** rather than accumulating, so a
 * suspended tile is correct on its next paint; and it is scheduled only while a session is open
 * **and** the window is visible. Hidden, occluded, or with no session it schedules nothing.
 *
 * The grammar is §7.8's published `2h 07m`, which fixes the zero-padding of minutes when hours
 * are present. The other ledger's `formatPlaytime` produces the same string over playtime; the
 * two are pinned by test and share no code, because one function across both ledgers is how they
 * get summed by accident.
 */
export const BENCH_LABEL = 'AT THE BENCH';

export function formatBenchElapsed(seconds: number): string {
  const totalMinutes = Math.max(0, Math.floor(seconds / 60));
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  if (hours === 0) return `${String(minutes)}m`;
  return `${String(hours)}h ${String(minutes).padStart(2, '0')}m`;
}

/** Milliseconds until the displayed minute changes. Never a fixed interval. */
export function benchTickDelayMs(elapsedSeconds: number): number {
  const remainder = Math.max(0, Math.floor(elapsedSeconds)) % 60;
  return (60 - remainder) * 1000;
}

/**
 * `nowSecs` must read a clock that is **current at the moment it is called**, not a value frozen
 * for the caller's paint: the wakeup below re-renders this card and nothing else, so a frozen
 * reader returns the same second it did a minute ago and the row stands still while its timer
 * keeps firing.
 */
export function useBenchElapsed(startedAt: number | null, nowSecs: () => number): string | null {
  const active = useWindowActive();
  const [, setWakeups] = useState(0);

  const elapsed = startedAt === null ? null : Math.max(0, nowSecs() - startedAt);

  useEffect(() => {
    if (elapsed === null || !active) return undefined;
    // The wakeup repaints this card and nothing else; the repaint re-reads the clock, `elapsed`
    // moves, and that is what arms the next boundary. The counter exists only to cause the
    // render — putting it in the dependency list as well buys nothing and costs a wakeup a
    // minute forever against a reader whose value never moves.
    const timer = setTimeout(() => {
      setWakeups((n) => n + 1);
    }, benchTickDelayMs(elapsed));
    return (): void => {
      clearTimeout(timer);
    };
  }, [elapsed, active]);

  return elapsed === null ? null : formatBenchElapsed(elapsed);
}

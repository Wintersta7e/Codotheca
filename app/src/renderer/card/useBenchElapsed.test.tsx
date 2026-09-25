import { act, cleanup, render, screen } from '@testing-library/react';
import type { ReactElement } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  BENCH_LABEL,
  benchTickDelayMs,
  formatBenchElapsed,
  useBenchElapsed,
} from './useBenchElapsed';

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

// jsdom's `document.hasFocus()` is FALSE by default, so `useWindowActive` reports inactive and
// nothing here would schedule anything — every scheduling assertion below would pass for the
// wrong reason. The activity gate gets its own tests; these pin the state they claim.
beforeEach(() => {
  vi.spyOn(document, 'hasFocus').mockReturnValue(true);
});

function Probe(props: {
  readonly startedAt: number | null;
  readonly now: () => number;
}): ReactElement {
  const text = useBenchElapsed(props.startedAt, props.now);
  return <span data-testid="bench">{text ?? 'no session'}</span>;
}

const bench = (): string => screen.getByTestId('bench').textContent;

describe('§7.8‘s figure: elapsed wall time, minute resolution', () => {
  it('formats <h>h <mm>m above an hour and <m>m below it', () => {
    expect(formatBenchElapsed(2 * 3600 + 7 * 60)).toBe('2h 07m');
    expect(formatBenchElapsed(7 * 60)).toBe('7m');
    expect(formatBenchElapsed(0)).toBe('0m');
    expect(formatBenchElapsed(59)).toBe('0m');
  });

  it('never prints a broken clock — 0h 07m is not a form this produces', () => {
    for (let m = 0; m < 60; m += 1) expect(formatBenchElapsed(m * 60)).not.toContain('h');
  });

  it('pads the minutes only when hours are present, which is §7.8‘s published form', () => {
    expect(formatBenchElapsed(3600 + 5 * 60)).toBe('1h 05m');
    expect(formatBenchElapsed(5 * 60)).toBe('5m');
  });

  it('re-derives from started_at rather than accumulating, so a suspended tile is correct', () => {
    const now = vi.fn(() => 1_000 + 3 * 3600 + 4 * 60);
    render(<Probe startedAt={1_000} now={now} />);
    expect(bench()).toBe('3h 04m');
  });

  it('renders nothing at all when no session is open', () => {
    render(<Probe startedAt={null} now={() => 5_000} />);
    expect(bench()).toBe('no session');
  });

  it('shows 0m rather than a negative figure if the clock is behind started_at', () => {
    render(<Probe startedAt={9_000} now={() => 5_000} />);
    expect(bench()).toBe('0m');
  });

  // The two ledgers are never merged, so the figure is labelled and the label is a constant the
  // card cannot restate differently.
  it('carries a label of its own and no word that reads as a total', () => {
    expect(BENCH_LABEL).toBe('AT THE BENCH');
    expect(BENCH_LABEL.toLowerCase()).not.toContain('total');
    expect(BENCH_LABEL.toLowerCase()).not.toContain('playtime');
  });
});

describe('criterion 21: it repaints at most once a minute and schedules nothing else', () => {
  it('waits out the remainder of the current minute, never a fixed interval', () => {
    expect(benchTickDelayMs(0)).toBe(60_000);
    expect(benchTickDelayMs(59)).toBe(1_000);
    expect(benchTickDelayMs(3 * 60 + 12)).toBe(48_000);
  });

  it('schedules a timer while open and none at all with no session', () => {
    vi.useFakeTimers();
    const open = render(<Probe startedAt={0} now={() => 90} />);
    expect(vi.getTimerCount()).toBeGreaterThan(0);
    open.unmount();

    render(<Probe startedAt={null} now={() => 90} />);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('schedules nothing while the window is hidden or unfocused', () => {
    vi.useFakeTimers();
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    render(<Probe startedAt={0} now={() => 90} />);
    expect(vi.getTimerCount()).toBe(0);
    // The figure is still correct on the frame it is on: suspended is not blank.
    expect(bench()).toBe('1m');
  });

  it('stops scheduling when the window loses focus and resumes when it returns', () => {
    vi.useFakeTimers();
    const focus = vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    render(<Probe startedAt={0} now={() => 90} />);
    expect(vi.getTimerCount()).toBeGreaterThan(0);

    focus.mockReturnValue(false);
    act(() => {
      window.dispatchEvent(new Event('blur'));
    });
    expect(vi.getTimerCount()).toBe(0);

    focus.mockReturnValue(true);
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    expect(vi.getTimerCount()).toBeGreaterThan(0);
  });

  it('advances one minute per wakeup', () => {
    vi.useFakeTimers();
    let clock = 100;
    render(<Probe startedAt={100} now={() => clock} />);
    expect(bench()).toBe('0m');
    act(() => {
      clock += 60;
      vi.advanceTimersByTime(60_000);
    });
    expect(bench()).toBe('1m');
    act(() => {
      clock += 60;
      vi.advanceTimersByTime(60_000);
    });
    expect(bench()).toBe('2m');
  });

  it('keeps re-arming across many minutes rather than stopping after the first', () => {
    // A timer armed once and never re-armed passes a single-advance test and freezes the row
    // at 1m for the rest of the session.
    vi.useFakeTimers();
    let clock = 0;
    render(<Probe startedAt={0} now={() => clock} />);
    for (let i = 0; i < 70; i += 1) {
      act(() => {
        clock += 60;
        vi.advanceTimersByTime(60_000);
      });
    }
    expect(bench()).toBe('1h 10m');
  });
});

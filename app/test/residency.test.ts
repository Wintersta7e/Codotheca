import { describe, expect, it, vi } from 'vitest';
import {
  HIDDEN_RESCAN_POLL_MS,
  RESCAN_DUE_AFTER_MS,
  RESIDENT_WINDOW_TTL_MS,
  Residency,
  rescanIsDue,
  trayMenu,
} from '../src/main/residency';
import type { ScanStatus } from '../src/generated/protocol';

interface Entry {
  fn: () => void;
  ms: number;
  cancelled: boolean;
  fired: boolean;
}

interface FakeTimers {
  readonly queue: Entry[];
  readonly setTimer: (fn: () => void, ms: number) => () => void;
  readonly fire: (ms: number) => void;
}

function fakeTimers(): FakeTimers {
  const queue: Entry[] = [];
  return {
    queue,
    setTimer: (fn: () => void, ms: number): (() => void) => {
      const entry: Entry = { fn, ms, cancelled: false, fired: false };
      queue.push(entry);
      return (): void => {
        entry.cancelled = true;
      };
    },
    /**
     * One-shot, like `setTimeout`: an entry fires at most once, and a timer scheduled *by* a
     * callback is not fired by the same call. Both matter here, because the destroy TTL and
     * the hidden poll are the same number of milliseconds.
     */
    fire: (ms: number): void => {
      for (const e of [...queue]) {
        if (!e.cancelled && !e.fired && e.ms === ms) {
          e.fired = true;
          e.fn();
        }
      }
    },
  };
}

const idle: ScanStatus = {
  runId: null,
  running: false,
  generation: null,
  mode: null,
  startedAt: null,
  endedAt: null,
  cancelled: false,
  walkedDirs: 0,
  foundRepos: 0,
  indexedProjects: 0,
  problemCount: null,
  ambiguousLineageCount: null,
};

describe('residency', () => {
  it('hides on close and destroys the window thirty minutes later', () => {
    const t = fakeTimers();
    const hideWindow = vi.fn();
    const destroyWindow = vi.fn();
    const r = new Residency({
      hideWindow,
      destroyWindow,
      showWindow: vi.fn(),
      isWindowAlive: () => true,
      setTimer: t.setTimer,
      now: () => 0,
      request: vi.fn(() => Promise.resolve(idle)),
    });
    r.onCloseRequested();
    expect(hideWindow).toHaveBeenCalledTimes(1);
    expect(destroyWindow).not.toHaveBeenCalled();
    t.fire(RESIDENT_WINDOW_TTL_MS);
    expect(destroyWindow).toHaveBeenCalledTimes(1);
  });

  it('showing the window inside the window cancels the destruction', () => {
    const t = fakeTimers();
    const destroyWindow = vi.fn();
    const r = new Residency({
      hideWindow: vi.fn(),
      destroyWindow,
      showWindow: vi.fn(),
      isWindowAlive: () => true,
      setTimer: t.setTimer,
      now: () => 0,
      request: vi.fn(() => Promise.resolve(idle)),
    });
    r.onCloseRequested();
    r.onShown();
    t.fire(RESIDENT_WINDOW_TTL_MS);
    expect(destroyWindow).not.toHaveBeenCalled();
  });

  it('closing twice leaves one destruction timer, not two', () => {
    const t = fakeTimers();
    const destroyWindow = vi.fn();
    const r = new Residency({
      hideWindow: vi.fn(),
      destroyWindow,
      showWindow: vi.fn(),
      isWindowAlive: () => true,
      setTimer: t.setTimer,
      now: () => 0,
      request: vi.fn(() => Promise.resolve(idle)),
    });
    r.onCloseRequested();
    r.onCloseRequested();
    expect(r.timers).toBe(1);
    t.fire(RESIDENT_WINDOW_TTL_MS);
    expect(destroyWindow).toHaveBeenCalledTimes(1);
  });

  it('leaves zero live timers once the app is quitting', () => {
    const t = fakeTimers();
    const r = new Residency({
      hideWindow: vi.fn(),
      destroyWindow: vi.fn(),
      showWindow: vi.fn(),
      isWindowAlive: () => false,
      setTimer: t.setTimer,
      now: () => 0,
      request: vi.fn(() => Promise.resolve(idle)),
    });
    r.onCloseRequested();
    r.onQuit();
    expect(r.timers).toBe(0);
  });

  it('starts the hidden poll only once the window is actually gone', async () => {
    const t = fakeTimers();
    const request = vi.fn(() => Promise.resolve(idle));
    const r = new Residency({
      hideWindow: vi.fn(),
      destroyWindow: vi.fn(),
      showWindow: vi.fn(),
      isWindowAlive: () => false,
      setTimer: t.setTimer,
      now: () => RESCAN_DUE_AFTER_MS * 5,
      request,
    });
    r.onCloseRequested();
    // While it is only hidden, the shell asks the core nothing.
    expect(request).not.toHaveBeenCalled();
    t.fire(RESIDENT_WINDOW_TTL_MS);
    t.fire(HIDDEN_RESCAN_POLL_MS);
    await vi.waitFor(() => {
      expect(request).toHaveBeenCalledWith('scan.status', {});
    });
    await vi.waitFor(() => {
      expect(request).toHaveBeenCalledWith('scan.start', { full: false });
    });
  });

  it('a rescan is due only when the last run ended more than a day ago', () => {
    const day = 86_400_000;
    expect(rescanIsDue({ ...idle, endedAt: 0 }, day + 1)).toBe(true);
    expect(rescanIsDue({ ...idle, endedAt: Math.floor(day / 1000) }, day + 1)).toBe(false);
    expect(rescanIsDue({ ...idle, running: true, endedAt: 0 }, day * 5)).toBe(false);
    expect(rescanIsDue({ ...idle, endedAt: null }, day * 5)).toBe(true);
  });

  it('reads endedAt as epoch seconds against a millisecond clock', () => {
    // The wire's only time unit is epoch seconds; `now` is a millisecond clock. Subtracting
    // them raw makes a scan from an hour ago look ~54 years old and rescans every launch.
    // These two figures are an hour apart, which is the only spacing that separates the two
    // readings — a day-scale gap is over the threshold either way and proves nothing.
    const nowMs = 1_700_000_000_000;
    const anHourAgoSecs = 1_699_996_400;
    expect(rescanIsDue({ ...idle, endedAt: anHourAgoSecs }, nowMs)).toBe(false);

    const twoDaysAgoSecs = 1_699_827_200;
    expect(rescanIsDue({ ...idle, endedAt: twoDaysAgoSecs }, nowMs)).toBe(true);
  });

  it('tray activation shows the shelf and opens nothing else', () => {
    const onActivate = vi.fn();
    const items = trayMenu({ onActivate, onQuit: vi.fn() });
    expect(items.map((i) => i.label)).toEqual(['SHOW CODOTHECA', 'QUIT']);
    items[0]?.click();
    expect(onActivate).toHaveBeenCalledTimes(1);
  });
});

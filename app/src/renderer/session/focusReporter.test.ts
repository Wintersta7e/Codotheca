import { describe, expect, it } from 'vitest';
import { FOCUS_HEARTBEAT_MS } from '../../shared/sessionFocus';
import { createFocusReporter, type FocusReporter } from './focusReporter';
import reporterSource from './focusReporter.ts?raw';

interface Harness {
  reporter: FocusReporter;
  sent: (number | null)[];
  fire: () => void;
  pending: () => { fn: () => void; ms: number } | null;
}

function harness(): Harness {
  const sent: (number | null)[] = [];
  let pending: { fn: () => void; ms: number } | null = null;
  let next = 1;
  const reporter = createFocusReporter({
    report: (args) => {
      sent.push(args.projectId);
      return Promise.resolve();
    },
    setTimer: (fn, ms) => {
      pending = { fn, ms };
      return next++;
    },
    clearTimer: () => {
      pending = null;
    },
  });
  return {
    reporter,
    sent,
    fire: () => {
      const p = pending;
      pending = null;
      p?.fn();
    },
    pending: () => pending,
  };
}

describe('the focus reporter', () => {
  it('a view alone claims nothing, across five heartbeats', () => {
    // §39.3a: a page left open with nobody at it must hold no segment open.
    const h = harness();
    h.reporter.focus(7);
    for (let beat = 0; beat < 5; beat += 1) h.fire();
    expect(h.sent).toEqual([]);
    expect(h.pending()).toBeNull();
    expect(h.reporter.claimed()).toBeNull();
  });

  it('trusted input claims the project at once and starts the heartbeat', () => {
    const h = harness();
    h.reporter.focus(7);
    h.reporter.input(7, true);
    expect(h.sent).toEqual([7]);
    expect(h.pending()?.ms).toBe(FOCUS_HEARTBEAT_MS);
    expect(h.reporter.claimed()).toBe(7);
  });

  it('further input inside a live claim sends nothing until the heartbeat', () => {
    const h = harness();
    h.reporter.input(7, true);
    h.reporter.input(7, true);
    h.reporter.input(7, true);
    expect(h.sent).toEqual([7]);
  });

  it('a heartbeat after input in the interval repeats the claim', () => {
    const h = harness();
    h.reporter.input(7, true);
    h.reporter.input(7, true);
    h.fire();
    expect(h.sent).toEqual([7, 7]);
    expect(h.pending()?.ms).toBe(FOCUS_HEARTBEAT_MS);
  });

  it('a heartbeat with no input in the interval sends nothing and stops', () => {
    // The core lets the claim lapse after FOCUS_STALE_SECS; repeating it would hold it open.
    const h = harness();
    h.reporter.input(7, true);
    h.fire();
    expect(h.sent).toEqual([7]);
    expect(h.pending()).toBeNull();
  });

  it('keeps beating for as long as input keeps arriving', () => {
    const h = harness();
    h.reporter.input(7, true);
    for (let beat = 0; beat < 3; beat += 1) {
      h.reporter.input(7, true);
      h.fire();
    }
    expect(h.sent).toEqual([7, 7, 7, 7]);
  });

  it('the first input after a lapse claims again at once', () => {
    const h = harness();
    h.reporter.input(7, true);
    h.fire();
    h.reporter.input(7, true);
    expect(h.sent).toEqual([7, 7]);
    expect(h.pending()?.ms).toBe(FOCUS_HEARTBEAT_MS);
  });

  it('untrusted input claims nothing', () => {
    const h = harness();
    h.reporter.focus(7);
    h.reporter.input(7, false);
    expect(h.sent).toEqual([]);
    expect(h.pending()).toBeNull();
  });

  it('untrusted input is no evidence for the heartbeat', () => {
    const h = harness();
    h.reporter.input(7, true);
    h.reporter.input(7, false);
    h.fire();
    expect(h.sent).toEqual([7]);
    expect(h.pending()).toBeNull();
  });

  it("a session starting discards that project's evidence, so the launch is not input", () => {
    const h = harness();
    h.reporter.input(7, true);
    h.reporter.input(7, true);
    h.reporter.sessionStarted(7);
    h.fire();
    expect(h.sent).toEqual([7]);
    expect(h.pending()).toBeNull();
  });

  it("another project's session starting leaves this one's evidence standing", () => {
    const h = harness();
    h.reporter.input(7, true);
    h.reporter.input(7, true);
    h.reporter.sessionStarted(9);
    h.fire();
    expect(h.sent).toEqual([7, 7]);
  });

  it('a view of another project releases the claim at once and claims nothing', () => {
    const h = harness();
    h.reporter.input(7, true);
    h.reporter.focus(9);
    expect(h.sent).toEqual([7, null]);
    expect(h.pending()).toBeNull();
    expect(h.reporter.claimed()).toBeNull();
  });

  it("the claimed project's own view releases nothing", () => {
    const h = harness();
    h.reporter.input(7, true);
    h.reporter.focus(7);
    expect(h.sent).toEqual([7]);
    expect(h.reporter.claimed()).toBe(7);
  });

  it('a lapsed claim is still released when the view changes', () => {
    // The core believes a lapsing claim until FOCUS_STALE_SECS, so the release is not redundant.
    const h = harness();
    h.reporter.input(7, true);
    h.fire();
    h.reporter.focus(null);
    expect(h.sent).toEqual([7, null]);
  });

  it('a view change with nothing claimed sends nothing', () => {
    const h = harness();
    h.reporter.focus(7);
    h.reporter.focus(null);
    expect(h.sent).toEqual([]);
  });

  it('input in another project switches the claim at once', () => {
    const h = harness();
    h.reporter.input(7, true);
    h.reporter.input(9, true);
    expect(h.sent).toEqual([7, 9]);
    h.fire();
    expect(h.sent).toEqual([7, 9]);
    expect(h.pending()).toBeNull();
  });

  it('stop releases the claim, so an unmount leaves nothing standing', () => {
    const h = harness();
    h.reporter.input(7, true);
    h.reporter.stop();
    expect(h.sent).toEqual([7, null]);
    expect(h.pending()).toBeNull();
    expect(h.reporter.claimed()).toBeNull();
  });

  it('stop with no claim held sends nothing', () => {
    const h = harness();
    h.reporter.stop();
    expect(h.sent).toEqual([]);
  });

  it('a rejected report does not stop the heartbeat', async () => {
    // A core restart is transient; permanently silencing the reporter would leave the next
    // session uncreditable for its whole life.
    let pending: (() => void) | null = null;
    let calls = 0;
    const reporter = createFocusReporter({
      report: () => {
        calls += 1;
        return Promise.reject(new Error('core restarting'));
      },
      setTimer: (fn) => {
        pending = fn;
        return 1;
      },
      clearTimer: () => {
        pending = null;
      },
    });
    reporter.input(7, true);
    reporter.input(7, true);
    await Promise.resolve();
    expect(pending).not.toBeNull();
    (pending as unknown as () => void)();
    expect(calls).toBe(2);
  });

  it('reads no clock', () => {
    // The core stamps every report with its own monotonic time; a renderer clock would be a
    // second one, and the reporter's timers are injected for the same reason.
    expect(reporterSource.length, 'focusReporter.ts read as an empty string').toBeGreaterThan(0);
    expect(/\bDate\.now\b|\bnew Date\b|\bperformance\.now\b/u.test(reporterSource)).toBe(false);
  });
});

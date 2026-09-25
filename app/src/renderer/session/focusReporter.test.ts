import { describe, expect, it } from 'vitest';
import { FOCUS_HEARTBEAT_MS } from '../../shared/sessionFocus';
import { createFocusReporter, type FocusReporter } from './focusReporter';

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
  it('sends a claim as soon as it is made', () => {
    const h = harness();
    h.reporter.focus(7);
    expect(h.sent).toEqual([7]);
  });

  it('does not resend an unchanged claim', () => {
    const h = harness();
    h.reporter.focus(7);
    h.reporter.focus(7);
    h.reporter.focus(7);
    expect(h.sent).toEqual([7]);
  });

  it('repeats a held claim on the heartbeat, at the shared interval', () => {
    const h = harness();
    h.reporter.focus(7);
    expect(h.pending()?.ms).toBe(FOCUS_HEARTBEAT_MS);
    h.fire();
    expect(h.sent).toEqual([7, 7]);
  });

  it('keeps beating for as long as the claim is held', () => {
    // One repeat proves the timer fired; three prove it rearms, which is what liveness needs.
    const h = harness();
    h.reporter.focus(7);
    h.fire();
    h.fire();
    h.fire();
    expect(h.sent).toEqual([7, 7, 7, 7]);
  });

  it('releasing the claim sends one null and stops the heartbeat', () => {
    // Nothing is being claimed, so there is nothing to keep proving.
    const h = harness();
    h.reporter.focus(7);
    h.reporter.focus(null);
    expect(h.sent).toEqual([7, null]);
    expect(h.pending()).toBeNull();
  });

  it('switching projects sends the new claim at once and restarts the heartbeat', () => {
    const h = harness();
    h.reporter.focus(7);
    h.reporter.focus(9);
    expect(h.sent).toEqual([7, 9]);
    h.fire();
    expect(h.sent).toEqual([7, 9, 9]);
  });

  it('stop releases the claim, so an unmount leaves nothing standing', () => {
    const h = harness();
    h.reporter.focus(7);
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
    reporter.focus(7);
    await Promise.resolve();
    expect(pending).not.toBeNull();
    (pending as unknown as () => void)();
    expect(calls).toBe(2);
  });

  it('reports the claim it currently holds', () => {
    const h = harness();
    expect(h.reporter.claimed()).toBeNull();
    h.reporter.focus(7);
    expect(h.reporter.claimed()).toBe(7);
  });
});

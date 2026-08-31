import { act, cleanup, render, screen } from '@testing-library/react';
import type { ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { CollectionId } from '../../generated/protocol.js';
import { DISARM_MS, useArmedDelete, type ArmedDeleteDeps } from './armedDelete.js';

afterEach(cleanup);

const A = 1 as CollectionId;

function harness(deps: ArmedDeleteDeps): { press: () => void } {
  const captured = { press: (): void => undefined };
  function Probe(): ReactElement {
    const armed = useArmedDelete(deps);
    captured.press = () => {
      armed.press(A);
    };
    return <span data-testid="armed">{armed.armedId === null ? 'no' : 'yes'}</span>;
  }
  render(<Probe />);
  return captured;
}

const armedText = (): string | null => screen.getByTestId('armed').textContent;

describe('useArmedDelete', () => {
  it('schedules one disarm, cancels it on commit, and commits through onCommit', () => {
    let now = 0;
    const timers = new Map<number, () => void>();
    let next = 1;
    const onCommit = vi.fn();
    const cleared: number[] = [];
    const probe = harness({
      nowMs: () => now,
      setTimer: (cb, ms) => {
        expect(ms).toBe(DISARM_MS);
        const handle = next++;
        timers.set(handle, cb);
        return handle;
      },
      clearTimer: (handle) => {
        cleared.push(handle);
        timers.delete(handle);
      },
      onCommit,
    });

    act(() => {
      probe.press();
    });
    expect(armedText()).toBe('yes');
    expect(timers.size).toBe(1);

    now = 500;
    act(() => {
      probe.press();
    });
    expect(onCommit).toHaveBeenCalledWith(A);
    expect(armedText()).toBe('no');
    expect(cleared).toHaveLength(1);
  });

  it('disarms itself when the scheduled sweep fires', () => {
    let now = 0;
    const scheduled: (() => void)[] = [];
    const probe = harness({
      nowMs: () => now,
      setTimer: (cb) => {
        scheduled.push(cb);
        return 1;
      },
      clearTimer: () => undefined,
      onCommit: vi.fn(),
    });
    act(() => {
      probe.press();
    });
    now = DISARM_MS + 1;
    act(() => {
      for (const fire of scheduled) fire();
    });
    expect(armedText()).toBe('no');
  });

  // The timer is the courtesy; the clock is the rule. A throttled renderer can fire the sweep
  // early, and an early sweep must leave the arm standing rather than cancelling a live one.
  it('keeps the arm when the sweep fires before the window has passed', () => {
    let now = 0;
    const scheduled: (() => void)[] = [];
    const probe = harness({
      nowMs: () => now,
      setTimer: (cb) => {
        scheduled.push(cb);
        return 1;
      },
      clearTimer: () => undefined,
      onCommit: vi.fn(),
    });
    act(() => {
      probe.press();
    });
    now = 10;
    act(() => {
      for (const fire of scheduled) fire();
    });
    expect(scheduled).toHaveLength(1);
    expect(armedText()).toBe('yes');
  });

  it('disarm clears both the state and the scheduled sweep', () => {
    const cleared: number[] = [];
    const captured = { disarm: (): void => undefined };
    function Probe(): ReactElement {
      const armed = useArmedDelete({
        nowMs: () => 0,
        setTimer: () => 7,
        clearTimer: (handle) => {
          cleared.push(handle);
        },
        onCommit: vi.fn(),
      });
      captured.disarm = armed.disarm;
      return (
        <button
          type="button"
          data-testid="armed"
          onClick={() => {
            armed.press(A);
          }}
        >
          {armed.armedId === null ? 'no' : 'yes'}
        </button>
      );
    }
    render(<Probe />);
    act(() => {
      screen.getByTestId('armed').click();
    });
    expect(armedText()).toBe('yes');
    act(() => {
      captured.disarm();
    });
    expect(armedText()).toBe('no');
    expect(cleared).toEqual([7]);
  });
});

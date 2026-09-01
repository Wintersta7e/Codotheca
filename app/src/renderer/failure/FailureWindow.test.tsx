import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ResolvedTier } from '../motion/tier';
import type { FailureFact } from './copy';
import { FAILURE_ENTER_MS, FailureWindow } from './FailureWindow';

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const SCHEMA: FailureFact = { kind: 'schema_from_future', onDisk: 9, supported: 5 };
const CORRUPT: FailureFact = {
  kind: 'corrupt_index',
  quarantinedAt: 1_700_000_000,
  gapStartedAt: null,
  gapCountsRecoverable: false,
  reDerivable: {
    projects: 212,
    notes: 0,
    sessions: 0,
    collections: 0,
    roots: 2,
    xpEvents: 0,
    launchTargets: 0,
  },
  restorable: {
    projects: 0,
    notes: 14,
    sessions: 96,
    collections: 3,
    roots: 0,
    xpEvents: 410,
    launchTargets: 6,
  },
};

interface Drawn {
  readonly onPrimary: ReturnType<typeof vi.fn>;
  readonly onSecondary: ReturnType<typeof vi.fn>;
}

function draw(
  fact: FailureFact,
  tier: ResolvedTier = 'full',
  nowMs: () => number = () => 0,
): Drawn {
  const onPrimary = vi.fn();
  const onSecondary = vi.fn();
  render(
    <FailureWindow
      fact={fact}
      logPath="/data/codotheca.log"
      tier={tier}
      nowMs={nowMs}
      onPrimary={onPrimary}
      onSecondary={onSecondary}
    />,
  );
  return { onPrimary, onSecondary };
}

describe('the style attribute survives jsdom', () => {
  // Every tier and token assertion below reads `getAttribute('style')`. If jsdom's CSS
  // serialiser dropped `var()` or the `animation` shorthand, those assertions would all read
  // '' and pass vacuously against a component wearing nothing.
  it('round-trips a custom property and an animation shorthand', () => {
    draw(SCHEMA, 'full');
    const root = screen.getByTestId('fw-root').getAttribute('style') ?? '';
    const eyebrow = screen.getByTestId('fw-eyebrow').getAttribute('style') ?? '';
    expect(root).toContain('animation');
    expect(eyebrow).toContain('var(');
  });
});

describe('the idiom', () => {
  it('carries the eyebrow, headline, body and log path', () => {
    draw(SCHEMA);
    expect(screen.getByTestId('fw-eyebrow').textContent).toBe('INDEX VERSION');
    expect(screen.getByTestId('fw-headline').textContent).toBe(
      'THIS LIBRARY WAS WRITTEN BY A NEWER CODOTHECA',
    );
    expect(screen.getByTestId('fw-log').textContent).toBe('LOG · /data/codotheca.log');
  });

  it('wears --fail-hot on the eyebrow and never --sig (§11.2a)', () => {
    draw(SCHEMA);
    const eyebrow = screen.getByTestId('fw-eyebrow');
    expect(eyebrow.getAttribute('style')).toContain('--fail-hot');
    expect(eyebrow.getAttribute('style')).not.toContain('--sig');
  });

  it('puts focus on the primary so a keyboard reaches it in the first frame', () => {
    draw(SCHEMA);
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'QUIT' }));
  });

  it('offers no secondary where the window has none', () => {
    draw(SCHEMA);
    expect(screen.getAllByRole('button')).toHaveLength(1);
  });
});

describe('the motion tier, which may be off because the GPU is what broke', () => {
  it('enters at §11.2a’s 300ms at full', () => {
    draw(SCHEMA, 'full');
    expect(screen.getByTestId('fw-root').getAttribute('style')).toContain(
      `viewIn ${String(FAILURE_ENTER_MS)}ms`,
    );
  });

  it('clamps to the reduced ceiling at reduced', () => {
    draw(SCHEMA, 'reduced');
    expect(screen.getByTestId('fw-root').getAttribute('style')).toContain('viewIn 160ms');
  });

  it('declares no animation at off', () => {
    draw(SCHEMA, 'off');
    expect(screen.getByTestId('fw-root').getAttribute('style') ?? '').not.toContain('viewIn');
  });
});

describe('the corrupt-index ledger', () => {
  it('draws three labelled blocks and prints no figure for an uncountable gap', () => {
    draw(CORRUPT);
    const labels = screen.getAllByTestId('fw-block-label').map((n) => n.textContent);
    expect(labels).toEqual([
      'RE-DERIVED FROM DISK',
      'RESTORED FROM THE SIDECAR',
      'LOST IN THE GAP',
    ]);
    const gap = screen.getByTestId('fw-block-2').textContent ?? '';
    expect(gap).toContain('not known');
    expect(gap).toContain('cannot be counted');
  });

  it('never sums the two ledgers into one figure (two ledgers, never merged)', () => {
    draw(CORRUPT);
    expect(screen.queryByTestId('fw-block-total')).toBeNull();
  });
});

describe('still shutting down', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  it('shows real elapsed seconds read from the clock, not a tick counter', () => {
    let now = 0;
    draw({ kind: 'still_shutting_down', startedAtMs: 0 }, 'full', () => now);
    expect(screen.getByTestId('fw-note').textContent).toBe('still shutting down · 0s');
    now = 7_000;
    act(() => {
      vi.advanceTimersByTime(1_000);
    }); // one tick, seven seconds of wall clock
    expect(screen.getByTestId('fw-note').textContent).toBe('still shutting down · 7s');
  });

  it('withholds FORCE until ten seconds have actually passed, then outlines it', () => {
    let now = 0;
    const { onPrimary, onSecondary } = draw(
      { kind: 'still_shutting_down', startedAtMs: 0 },
      'full',
      () => now,
    );
    expect(screen.getByRole('button', { name: 'WAIT' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'FORCE' })).toBeNull();

    now = 10_000;
    act(() => {
      vi.advanceTimersByTime(1_000);
    });

    const force = screen.getByRole('button', { name: 'FORCE' });
    // The filled --sig bar is the safe action everywhere in this product.
    expect(force.getAttribute('style')).not.toContain('--sig');
    expect(screen.getByRole('button', { name: 'WAIT' }).getAttribute('style')).toContain('--sig');

    fireEvent.click(force);
    expect(onSecondary).toHaveBeenCalledTimes(1);
    expect(onPrimary).not.toHaveBeenCalled();
  });
});

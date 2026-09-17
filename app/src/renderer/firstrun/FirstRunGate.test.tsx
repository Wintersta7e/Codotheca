import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import { FirstRunGate, gateDecision } from './FirstRunGate';
import { TurnScreen } from './TurnScreen';
import type { FirstRunGateDeps } from './FirstRunGate';
import { SETTLE_HOLD_MS } from './phase';
import { SKIP_AHEAD_LABEL } from '../a11y/names';
import * as copy from './copy';
import type { Mock } from 'vitest';
import type { PickRootReply } from '../../shared/channels';
import type { ScanFeedEvent } from './scanFeed';
import type {
  ProjectId,
  Reveal,
  RootAdd,
  RootSuggestion,
  ScanRunId,
  ScanStatus,
} from '../../generated/protocol';

/**
 * The mocks keep their real signatures. Widening them to `ReturnType<typeof vi.fn>` makes every
 * one a void-returning procedure, and `mockImplementation` then rejects the promise-returning
 * bodies these deps actually have — which would have been "fixed" by dropping the assertions.
 */
interface Harness {
  /** Every query `SHOW ME` handed up, in order. Empty until the turn's button is pressed. */
  readonly shownQueries: string[];
  readonly deps: FirstRunGateDeps & {
    readonly suggestRoots: Mock<() => Promise<readonly RootSuggestion[]>>;
    readonly commitSuggestion: Mock<(pathDisplay: string) => Promise<RootAdd>>;
    readonly pickRoot: Mock<(confirmLarge: boolean) => Promise<PickRootReply>>;
    readonly startScan: Mock<() => Promise<void>>;
    readonly loadReveal: Mock<() => Promise<Reveal>>;
  };
  readonly emit: (event: ScanFeedEvent) => void;
  readonly released: Mock<() => void>;
  readonly unmount: () => void;
  /** Re-render with new props, which is how the core's own status reaches a mounted gate. */
  readonly setStatus: (next: ScanStatus | null) => void;
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const status = (runId: number | null): ScanStatus => ({
  runId: runId === null ? null : (runId as ScanRunId),
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
});

const suggestion: RootSuggestion = {
  pathDisplay: '/somewhere/dev',
  kind: 'linux',
  distro: '',
  provenance: 'gitconfig',
  provenanceDetail: 'includeif',
  hits: 3,
  preTicked: true,
};

const basis = { projectsCovered: 1, projectsTotal: 1, historyComplete: true };
const reveal: Reveal = {
  spanDays: { value: 4_400, basis },
  projectCount: { value: 1, basis },
  languageCount: { value: 1, basis },
  bestYear: { value: 2021, basis },
  playtimeSeconds: { value: 0, basis },
  oldestStillAlive: { projectId: 1 as ProjectId, firstCommitAt: 1, basis },
};

const noRefusal = { root: null, refusedBecause: null, estimatedDirs: null };

function harness(over: Partial<Parameters<typeof FirstRunGate>[0]> = {}): Harness {
  let emit: (event: ScanFeedEvent) => void = () => undefined;
  const shownQueries: string[] = [];
  const released = vi.fn();
  const deps = {
    nowMs: () => Date.now(),
    suggestRoots: vi.fn(() => Promise.resolve([suggestion])),
    commitSuggestion: vi.fn(() => Promise.resolve(noRefusal)),
    pickRoot: vi.fn((): Promise<PickRootReply> => Promise.resolve({ kind: 'cancelled' })),
    startScan: vi.fn(() => Promise.resolve(undefined)),
    loadReveal: vi.fn(() => Promise.resolve(reveal)),
    revealDeps: {
      nowSecs: 1_760_000_000,
      languageTally: [],
      referenceCount: 0,
      project: () => null,
    },
    scan: {
      jewelFor: () => null,
      onSkipAhead: () => undefined,
      onOpenScanSummary: () => undefined,
    },
    subscribe: (cb: (event: ScanFeedEvent) => void) => {
      emit = cb;
      return released;
    },
    // GAP-16b-4, closed: the gate takes the beat and plan 16c's `TurnScreen` is what fills it.
    // The seam is exercised against the production component rather than a stand-in, so a
    // handler shape that only a fake satisfies cannot pass here. The query is closed over at
    // this call site because the gate has nowhere to put it — the shelf is what consumes it.
    renderTurn: (h: { onShowMe: () => void; onNotNow: () => void }) => (
      <TurnScreen
        counts={{ unpushed: 4, dirty: 0, interrupted: 0, total: 212 }}
        worktreeObservedAt={null}
        tier="full"
        onShowMe={(query) => {
          shownQueries.push(query);
          h.onShowMe();
        }}
        onNotNow={h.onNotNow}
      />
    ),
    rootLine: `/somewhere/dev · 1 ${copy.ROOTS_SUFFIX}`,
  };
  const view = render(
    <FirstRunGate deps={deps} status={status(null)} hasStoredShelf={false} tier="full" {...over}>
      <div data-testid="shelf">the shelf</div>
    </FirstRunGate>,
  );
  return {
    shownQueries,
    deps,
    emit: (e: ScanFeedEvent) => {
      act(() => {
        emit(e);
      });
    },
    released,
    unmount: view.unmount,
    setStatus: (next: ScanStatus | null) => {
      view.rerender(
        <FirstRunGate deps={deps} status={next} hasStoredShelf={false} tier="full" {...over}>
          <div data-testid="shelf">the shelf</div>
        </FirstRunGate>,
      );
    },
  };
}

const dig = (): void => {
  fireEvent.click(screen.getByRole('button', { name: copy.DIG_LABEL }));
};

// Criterion 15 times process start → first tile pixel, and §11.2 paints from the stored
// snapshot before the core joins. A stored shelf is proof first run is over.
test('a stored shelf is never held for a status round trip', () => {
  expect(gateDecision(null, true)).toBe('shelf');
  expect(gateDecision(null, false)).toBe('wait');
  expect(gateDecision(status(7), false)).toBe('shelf');
  expect(gateDecision(status(null), false)).toBe('first-run');
  harness({ status: null, hasStoredShelf: true });
  expect(screen.getByTestId('shelf')).toBeTruthy();
});

// §10.5: the reveal never replays. A scan run exists from the moment DIG is pressed and never
// goes away.
test('a library that has scanned once goes straight to the shelf', () => {
  harness({ status: status(7) });
  expect(screen.getByTestId('shelf')).toBeTruthy();
  expect(screen.queryByText(copy.ROOTS_HEADLINE)).toBeNull();
});

// The decision is an **entry** condition. `DIG` creates the very run that `gateDecision` reads as
// proof first run is over, so re-deciding on every render ended first run from inside first run —
// measured in the real app, the scanning screen, the reveal and the turn were gone within a second
// of `DIG`. The test above mounts with a run already present and could never see it.
test('a run that first run itself started does not end first run', async () => {
  const view = harness();
  await screen.findByText('/somewhere/dev');
  dig();
  await waitFor(() => {
    expect(view.deps.startScan).toHaveBeenCalledTimes(1);
  });

  // What `scan/run_started` does to the status the host holds.
  act(() => {
    view.setStatus(status(7));
  });
  expect(screen.queryByTestId('shelf')).toBeNull();

  // And the walk finishing does not release it either; the beats own the screen to their end.
  act(() => {
    view.setStatus({ ...status(7), endedAt: 1_760_000_100, indexedProjects: 3 });
  });
  expect(screen.queryByTestId('shelf')).toBeNull();
});

// §11.2a: every full-screen flow unmounts the shelf. 522 MB of cards behind a screen nobody can
// see is paid for nothing.
test('the shelf is unmounted, not hidden, while a beat is on screen', async () => {
  harness();
  await screen.findByText(copy.ROOTS_HEADLINE);
  expect(screen.queryByTestId('shelf')).toBeNull();
});

test('the gate asks the core for suggestions once and draws them with their ticks', async () => {
  const { deps } = harness();
  await screen.findByText('/somewhere/dev');
  expect(deps.suggestRoots).toHaveBeenCalledTimes(1);
  // `preTicked` is the core's, so the drawn tick is the core's answer and not a default.
  const row = screen.getByTestId('fr-root-row');
  expect(row.getAttribute('aria-checked')).toBe('true');
});

// §2.4: the renderer may never originate a filesystem path. Nothing but a display string the
// core produced crosses back.
test('committing a tick sends the display string the core produced, and nothing else', async () => {
  const { deps } = harness();
  await screen.findByText('/somewhere/dev');
  dig();
  await waitFor(() => {
    expect(deps.commitSuggestion).toHaveBeenCalledWith('/somewhere/dev');
  });
  await waitFor(() => {
    expect(deps.startScan).toHaveBeenCalledTimes(1);
  });
});

// The commit must precede the start, or the walk has no roots.
test('the scan starts only after every ticked root is committed', async () => {
  const order: string[] = [];
  const { deps } = harness();
  deps.commitSuggestion.mockImplementation(() => {
    order.push('commit');
    return Promise.resolve(noRefusal);
  });
  deps.startScan.mockImplementation(() => {
    order.push('start');
    return Promise.resolve(undefined);
  });
  await screen.findByText('/somewhere/dev');
  dig();
  await waitFor(() => {
    expect(order).toEqual(['commit', 'start']);
  });
});

// §10.1b: unticking row 1 is honoured — DIG goes inert and nothing is committed.
test('withholding consent commits nothing and starts nothing', async () => {
  const { deps } = harness();
  await screen.findByText(copy.ROOTS_HEADLINE);
  fireEvent.click(screen.getAllByRole('checkbox').at(-1)!);
  dig();
  expect(deps.commitSuggestion).not.toHaveBeenCalled();
  expect(deps.startScan).not.toHaveBeenCalled();
  expect(screen.getByText(copy.DIG_INERT_NOTE)).toBeTruthy();
});

// §2.4 and §10.1b: the dialog is the only way a folder reaches the core, and its three replies
// are three different events. A `failed` reply drawn as a row would put a folder on the list the
// user never picked and the core never accepted — which is what collapsing the reply down to
// `RootAdd | null` would have allowed.
test('only an accepted pick becomes a row; cancelling and failing draw nothing', async () => {
  const { deps } = harness();
  await screen.findByText(copy.ROOTS_HEADLINE);
  const rowCount = (): number => screen.getAllByTestId('fr-root-row').length;
  expect(rowCount()).toBe(1);

  fireEvent.click(screen.getByRole('button', { name: copy.ADD_A_FOLDER_LABEL }));
  await waitFor(() => {
    expect(deps.pickRoot).toHaveBeenCalledWith(false);
  });
  expect(rowCount()).toBe(1);

  deps.pickRoot.mockResolvedValue({
    kind: 'failed',
    // R31: the code is the schema's, not one invented here. `outcome: null` is "definitely did
    // not take effect", which is what a dialog that never opened is.
    error: { code: 'PERMISSION_DENIED', message: 'dialog failed', outcome: null, retryable: true },
  });
  fireEvent.click(screen.getByRole('button', { name: copy.ADD_A_FOLDER_LABEL }));
  await waitFor(() => {
    expect(deps.pickRoot).toHaveBeenCalledTimes(2);
  });
  expect(rowCount()).toBe(1);
  // The message never reaches the DOM raw — `CoreCallError`'s rule, applied to a shell reply.
  expect(document.body.textContent).not.toContain('dialog failed');

  deps.pickRoot.mockResolvedValue({
    kind: 'added',
    add: { root: null, refusedBecause: 'filesystem_root', estimatedDirs: null },
  });
  fireEvent.click(screen.getByRole('button', { name: copy.ADD_A_FOLDER_LABEL }));
  await waitFor(() => {
    expect(rowCount()).toBe(2);
  });
  expect(screen.getByText('A DRIVE ROOT IS NOT A PROJECT FOLDER')).toBeTruthy();
});

// §10.3: batches on a fixed ~600 ms cadence, never one per discovery.
test('discoveries land in batches, not one at a time', async () => {
  vi.useFakeTimers();
  const { emit } = harness();
  await act(async () => {});
  expect(screen.getByText(copy.ROOTS_HEADLINE)).toBeTruthy();
  dig();
  await act(async () => {});
  expect(screen.getByRole('button', { name: SKIP_AHEAD_LABEL })).toBeTruthy();
  emit({ kind: 'upserted', id: 1 as ProjectId, name: 'alpha', primaryLanguage: null });
  expect(screen.queryByText('alpha')).toBeNull();
  act(() => {
    vi.advanceTimersByTime(700);
  });
  expect(screen.getByText('alpha')).toBeTruthy();
});

// §10.3a: exactly one settle at walk completion, then 700 ms before the reveal takes the
// screen. The reveal must not pre-empt it.
test('the reveal waits out the settle hold and then loads once', async () => {
  vi.useFakeTimers();
  const { deps, emit } = harness();
  await act(async () => {});
  expect(screen.getByText(copy.ROOTS_HEADLINE)).toBeTruthy();
  dig();
  await act(async () => {});
  expect(screen.getByRole('button', { name: SKIP_AHEAD_LABEL })).toBeTruthy();
  emit({ kind: 'upserted', id: 1 as ProjectId, name: 'alpha', primaryLanguage: 'Rust' });
  emit({ kind: 'progress', indexedProjects: 1, walkedDirs: 10, foundRepos: 1 });
  emit({ kind: 'finished' });
  act(() => {
    vi.advanceTimersByTime(SETTLE_HOLD_MS - 1);
  });
  expect(screen.queryByText(copy.EVIDENCE_FOOTER)).toBeNull();
  act(() => {
    vi.advanceTimersByTime(2);
  });
  await act(async () => {});
  expect(screen.getByText(copy.EVIDENCE_FOOTER)).toBeTruthy();
  expect(deps.loadReveal).toHaveBeenCalledTimes(1);
});

// §10.3a: SKIP AHEAD reaches the reveal immediately — the walk keeps running behind it, which
// is the only thing the DIG note claims.
test('skip ahead reaches the reveal without waiting for the walk', async () => {
  const { deps } = harness();
  await screen.findByText(copy.ROOTS_HEADLINE);
  dig();
  fireEvent.click(await screen.findByRole('button', { name: SKIP_AHEAD_LABEL }));
  await waitFor(() => {
    expect(deps.loadReveal).toHaveBeenCalled();
  });
  await screen.findByText(copy.EVIDENCE_FOOTER);
});

// §10.4a: a shelf with no projects never reaches the reveal and gets §11.1 instead.
test('an empty library skips the reveal and the turn entirely', async () => {
  vi.useFakeTimers();
  const { deps, emit } = harness();
  deps.loadReveal.mockResolvedValue({ ...reveal, projectCount: { value: 0, basis } });
  await act(async () => {});
  expect(screen.getByText(copy.ROOTS_HEADLINE)).toBeTruthy();
  dig();
  await act(async () => {});
  expect(screen.getByRole('button', { name: SKIP_AHEAD_LABEL })).toBeTruthy();
  emit({ kind: 'finished' });
  act(() => {
    vi.advanceTimersByTime(SETTLE_HOLD_MS + 1);
  });
  await act(async () => {});
  expect(screen.getByTestId('shelf')).toBeTruthy();
  expect(screen.queryByText(copy.EVIDENCE_FOOTER)).toBeNull();
  expect(screen.queryByRole('button', { name: copy.SHOW_ME_LABEL })).toBeNull();
});

// §10.3a: a milestone renders one reveal figure computed at that instant, from the same call as
// the reveal itself.
test('crossing a milestone previews a figure from the reveal own basis', async () => {
  const { deps, emit } = harness();
  await screen.findByText(copy.ROOTS_HEADLINE);
  dig();
  await screen.findByRole('button', { name: SKIP_AHEAD_LABEL });
  emit({ kind: 'progress', indexedProjects: 10, walkedDirs: 1, foundRepos: 1 });
  await waitFor(() => {
    expect(deps.loadReveal).toHaveBeenCalled();
  });
  await waitFor(() => {
    expect(screen.getByText(copy.SO_FAR_QUALIFIER)).toBeTruthy();
  });
});

// §10.5: nothing returns to a beat that has already played.
test('the turn leads to the shelf and nothing leads back', async () => {
  const { deps } = harness();
  await screen.findByText(copy.ROOTS_HEADLINE);
  dig();
  fireEvent.click(await screen.findByRole('button', { name: SKIP_AHEAD_LABEL }));
  fireEvent.click(await screen.findByRole('button', { name: copy.GO_ON_LABEL }));
  fireEvent.click(await screen.findByRole('button', { name: copy.SHOW_ME_LABEL }));
  expect(screen.getByTestId('shelf')).toBeTruthy();
  expect(screen.queryByText(copy.EVIDENCE_FOOTER)).toBeNull();
  expect(deps.loadReveal).toHaveBeenCalledTimes(1);
});

test('the subscription is released when the gate stops needing it', async () => {
  const { released, unmount } = harness();
  await screen.findByText(copy.ROOTS_HEADLINE);
  dig();
  await screen.findByRole('button', { name: SKIP_AHEAD_LABEL });
  expect(released).not.toHaveBeenCalled();
  unmount();
  expect(released).toHaveBeenCalled();
});

// Criterion 12: first run asks zero configuration questions, shows no percentage, and states
// its coverage on any figure computed over a partial index.
test('AC-12 first run asks nothing, shows no percentage, and states its coverage', async () => {
  const partial = { projectsCovered: 212, projectsTotal: 400, historyComplete: false };
  const { deps } = harness();
  deps.loadReveal.mockResolvedValue({
    spanDays: { value: 4_400, basis: partial },
    projectCount: { value: 212, basis: partial },
    languageCount: { value: 9, basis: partial },
    bestYear: { value: 2021, basis: partial },
    playtimeSeconds: {
      value: 0,
      basis: { ...partial, projectsTotal: 212, historyComplete: true },
    },
    oldestStillAlive: { projectId: 1 as ProjectId, firstCommitAt: 1, basis: partial },
  });

  await screen.findByText(copy.ROOTS_HEADLINE);
  const seen: string[] = [];
  const sweep = (): void => {
    seen.push(document.body.textContent ?? '');
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.queryByRole('combobox')).toBeNull();
    expect(screen.queryByRole('radio')).toBeNull();
    expect(screen.queryByRole('progressbar')).toBeNull();
  };

  sweep();
  dig();
  await screen.findByRole('button', { name: SKIP_AHEAD_LABEL });
  sweep();
  fireEvent.click(screen.getByRole('button', { name: SKIP_AHEAD_LABEL }));
  await screen.findByText(copy.EVIDENCE_FOOTER);
  sweep();
  // Five of six figures over a partial index, each saying so. Read before the reveal is left.
  expect(screen.getAllByTestId('fr-panel-coverage')).toHaveLength(5);

  // The fourth beat. Report 16b left the sweep at three because the turn was 16c's and the
  // stand-in carried no copy; it carries the production screen now, so the criterion is
  // checked over the whole flow rather than over the three quarters of it that were drawn.
  fireEvent.click(screen.getByRole('button', { name: copy.GO_ON_LABEL }));
  await screen.findByRole('button', { name: copy.SHOW_ME_LABEL });
  sweep();
  // §10.4a: exactly two controls on the turn, and the identity and residency questions are
  // both on the shelf behind it — a third here would silently undo §1.4 and §11.3a.
  expect(screen.getAllByRole('button')).toHaveLength(2);

  expect(seen).toHaveLength(4);
  for (const text of seen) expect(text).not.toMatch(/%/);
});

// The turn's `SHOW ME` lands the shelf on the rung its own line was drawn from; the gate has
// nowhere to put a query, so the wiring site is what carries it and this pins that it does.
test('the query the turn computed reaches the host, and NOT NOW carries none', async () => {
  const { shownQueries } = harness();
  await screen.findByText(copy.ROOTS_HEADLINE);
  dig();
  fireEvent.click(await screen.findByRole('button', { name: SKIP_AHEAD_LABEL }));
  fireEvent.click(await screen.findByRole('button', { name: copy.GO_ON_LABEL }));
  expect(shownQueries).toEqual([]);
  fireEvent.click(await screen.findByRole('button', { name: copy.SHOW_ME_LABEL }));
  expect(shownQueries).toEqual(['is:unpushed']);
  expect(screen.getByTestId('shelf')).toBeTruthy();
});

// Criterion 23, reveal half: every reveal figure carries its coverage; shallow repositories are
// excluded from history statistics and counted in the coverage line.
test('AC-23 no figure over a partial index is presented bare', async () => {
  const partial = { projectsCovered: 212, projectsTotal: 400, historyComplete: false };
  const { deps } = harness();
  deps.loadReveal.mockResolvedValue({
    spanDays: { value: 4_400, basis: partial },
    projectCount: { value: 212, basis: partial },
    languageCount: { value: 9, basis: partial },
    bestYear: { value: 2021, basis: partial },
    playtimeSeconds: {
      value: 0,
      basis: { ...partial, projectsTotal: 212, historyComplete: true },
    },
    oldestStillAlive: { projectId: 1 as ProjectId, firstCommitAt: 1, basis: partial },
  });
  await screen.findByText(copy.ROOTS_HEADLINE);
  dig();
  fireEvent.click(await screen.findByRole('button', { name: SKIP_AHEAD_LABEL }));
  await screen.findByText(copy.EVIDENCE_FOOTER);

  const panels = screen.getAllByTestId('fr-panel');
  expect(panels).toHaveLength(6);
  for (const panel of panels) {
    const label = within(panel).getByTestId('fr-panel-value').textContent ?? '';
    const coverage = within(panel).queryByTestId('fr-panel-coverage');
    // PLAYTIME is the only figure complete by construction.
    if (panel.textContent?.includes('PLAYTIME') === true) expect(coverage).toBeNull();
    else expect(coverage, label).not.toBeNull();
  }
  // The two history strings are distinguishable: one promises growth, one admits movement.
  expect(screen.getByText(new RegExp(copy.COVERAGE_HISTORY_GROWS))).toBeTruthy();
  expect(screen.getAllByText(new RegExp(copy.COVERAGE_HISTORY_MOVES))).toHaveLength(2);
});

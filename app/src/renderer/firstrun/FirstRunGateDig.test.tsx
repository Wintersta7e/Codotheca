import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import type { ReactElement } from 'react';
import type { Mock } from 'vitest';
import { FirstRunGate } from './FirstRunGate';
import { suggestionKey } from './rootRows';
import type { RootsScreenProps } from './RootsScreen';
import type { FirstRunGateDeps } from './FirstRunGate';
import type { RootAdd, RootSuggestion, ScanStatus } from '../../generated/protocol';
import { required } from '../../shared/required';

/**
 * [p4] §43.10: the roots screen already refuses an inert DIG press before it reaches the gate, so
 * the gate's own guard is only observable when the handler is called directly. The screen is
 * replaced by a stand-in that hands its deps out; everything else is the production gate.
 */
const captured: { props: RootsScreenProps | null } = { props: null };

vi.mock('./RootsScreen', () => ({
  RootsScreen: (props: RootsScreenProps): ReactElement => {
    captured.props = props;
    return <div data-testid="roots-stand-in" />;
  },
}));

afterEach(() => {
  cleanup();
  captured.props = null;
});

/** No scan has ever run: the gate's entry condition for first run. */
const neverScanned: ScanStatus = {
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

const suggestion: RootSuggestion = {
  pathDisplay: '/somewhere/dev',
  kind: 'linux',
  distro: '',
  provenance: 'gitconfig',
  provenanceDetail: 'includeif',
  hits: 3,
  preTicked: false,
};

interface Mounted {
  readonly commitSuggestion: Mock<() => Promise<RootAdd>>;
  readonly startScan: Mock<() => Promise<undefined>>;
}

function mount(rows: Promise<readonly RootSuggestion[]>): Mounted {
  const commitSuggestion = vi.fn((): Promise<RootAdd> =>
    Promise.resolve({ root: null, refusedBecause: null, estimatedDirs: null }),
  );
  const startScan = vi.fn(() => Promise.resolve(undefined));
  const deps = {
    nowMs: () => Date.now(),
    suggestRoots: () => rows,
    commitSuggestion,
    pickRoot: () => Promise.resolve({ kind: 'cancelled' as const }),
    startScan,
    loadReveal: () => new Promise<never>(() => undefined),
    acknowledge: () => Promise.resolve(),
    listBadges: () => new Promise<never>(() => undefined),
    markLanded: () => Promise.resolve(),
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
    subscribe: () => () => undefined,
    renderTurn: () => null,
    rootLine: '',
  } satisfies FirstRunGateDeps;
  render(
    <FirstRunGate
      deps={deps}
      status={neverScanned}
      hasStoredShelf={false}
      tier="full"
      motivation={null}
    >
      <div />
    </FirstRunGate>,
  );
  return { commitSuggestion, startScan };
}

const roots = (): RootsScreenProps => required(captured.props, 'the roots screen rendered');

test('AC-P4-43-16 the DIG handler itself commits nothing while no root would be committed', async () => {
  let answer: (rows: readonly RootSuggestion[]) => void = () => undefined;
  const pending = new Promise<readonly RootSuggestion[]>((resolve) => {
    answer = resolve;
  });
  const { commitSuggestion, startScan } = mount(pending);
  await screen.findByTestId('roots-stand-in');

  // Before the suggestions answer.
  await act(async () => {
    roots().deps.onDig();
    await Promise.resolve();
  });
  expect(commitSuggestion).not.toHaveBeenCalled();
  expect(startScan).not.toHaveBeenCalled();

  // Answered, nothing ticked.
  await act(async () => {
    answer([suggestion]);
    await pending;
  });
  expect(roots().digInput.suggestionsAnswered).toBe(true);
  await act(async () => {
    roots().deps.onDig();
    await Promise.resolve();
  });
  expect(commitSuggestion).not.toHaveBeenCalled();
  expect(startScan).not.toHaveBeenCalled();

  // Ticked, but consent withheld.
  act(() => {
    roots().deps.onToggleRoot(suggestionKey(suggestion));
    roots().deps.onConsent(false);
  });
  expect(roots().digInput.ticked).toBe(1);
  await act(async () => {
    roots().deps.onDig();
    await Promise.resolve();
  });
  expect(commitSuggestion).not.toHaveBeenCalled();
  expect(startScan).not.toHaveBeenCalled();

  // The control: the same direct call with a root that would be committed reaches the core,
  // so the refusals above are the guard's and not a stand-in that never calls through.
  act(() => {
    roots().deps.onConsent(true);
  });
  await act(async () => {
    roots().deps.onDig();
    await Promise.resolve();
  });
  await vi.waitFor(() => {
    expect(startScan).toHaveBeenCalledTimes(1);
  });
  expect(commitSuggestion).toHaveBeenCalledTimes(1);
});

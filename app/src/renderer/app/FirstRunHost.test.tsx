import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ProjectId, Root, RootId, ScanStatus } from '../../generated/protocol';
import { NOT_NOW_LABEL, SHOW_ME_LABEL } from '../firstrun/copy';
import { TURN_QUERIES } from '../firstrun/turn';
import type { QueryContext } from '../shelf/evaluate';
import { projectionCapabilities, toShelfRow, type ShelfRow } from '../shelf/row';
import { makeProjectRow } from '../testing/projectRow';
import {
  FirstRunHost,
  newestWorktreeObservation,
  rootLineOf,
  toScanFeedEvent,
  TurnBeat,
  turnCountsOf,
} from './FirstRunHost';
import { fakeAppDeps, type FakeReplies } from './testDeps';

afterEach(() => {
  cleanup();
});

const NOW = 1_700_000_000;

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

const row = (over: Parameters<typeof makeProjectRow>[0] = {}): ShelfRow =>
  toShelfRow(makeProjectRow(over));

function context(rows: readonly ShelfRow[]): QueryContext {
  return {
    now: NOW,
    firstRunCompletedAt: null,
    collectionIdsByName: new Map(),
    pathsAreCaseSensitive: false,
    capabilities: projectionCapabilities(rows),
    commitSubjectHits: null,
  };
}

const root = (id: number, pathDisplay: string, enabled = true): Root => ({
  id: id as RootId,
  pathDisplay,
  kind: 'linux',
  distro: '',
  enabled,
  descendIntoRepos: false,
  provenance: 'gitconfig',
  state: 'watched',
  addedAt: NOW,
  projectCount: null,
});

function mount(
  replies: FakeReplies,
  over: { rows?: readonly ShelfRow[]; status?: ScanStatus | null; hasStoredShelf?: boolean } = {},
): void {
  const fake = fakeAppDeps(replies);
  fake.setNow(NOW);
  const rows = over.rows ?? [];
  render(
    <FirstRunHost
      deps={fake.deps}
      rows={rows}
      firstRunCompletedAt={null}
      status={over.status === undefined ? neverScanned : over.status}
      hasStoredShelf={over.hasStoredShelf ?? false}
      tier="off"
      onOpenScanSummary={vi.fn()}
      onShowMe={vi.fn()}
    >
      <div data-testid="shelf" />
    </FirstRunHost>,
  );
}

describe('rootLineOf', () => {
  it('names the enabled roots and counts them', () => {
    expect(rootLineOf([root(1, '/a'), root(2, '/b')])).toBe('/a · /b · 2 ROOTS');
  });

  it('says ROOT once, and nothing at all with none enabled', () => {
    expect(rootLineOf([root(1, '/a')])).toBe('/a · 1 ROOT');
    expect(rootLineOf([root(1, '/a', false)])).toBe('');
  });
});

describe('toScanFeedEvent', () => {
  it('carries the shapes the feed declares and drops the rest', () => {
    expect(
      toScanFeedEvent({
        topic: 'projects',
        event: 'upserted',
        data: { row: { id: 3, name: 'thing', primaryLanguage: 'Rust' } },
      }),
    ).toEqual({ kind: 'upserted', id: 3, name: 'thing', primaryLanguage: 'Rust' });

    expect(
      toScanFeedEvent({ topic: 'scan', event: 'progress', data: { indexedProjects: 2 } }),
    ).toEqual({ kind: 'progress', indexedProjects: 2, walkedDirs: 0, foundRepos: 0 });

    expect(toScanFeedEvent({ topic: 'scan', event: 'finished', data: {} })).toEqual({
      kind: 'finished',
    });
    // A cancelled walk ends the feed too: the beat must not sit waiting for a run that stopped.
    expect(toScanFeedEvent({ topic: 'scan', event: 'cancelled', data: {} })).toEqual({
      kind: 'finished',
    });
    expect(toScanFeedEvent({ topic: 'session', event: 'started', data: {} })).toBeNull();
    expect(toScanFeedEvent({ topic: 'scan', event: 'problem', data: {} })).toBeNull();
  });
});

describe('turnCountsOf', () => {
  it('counts the rungs with the shelf predicates, not a second set', () => {
    const rows = [
      row({ id: 1 as ProjectId, ahead: 2 }),
      row({ id: 2 as ProjectId, isDirty: true }),
      row({ id: 3 as ProjectId, interruptedOp: 'rebase' }),
      row({ id: 4 as ProjectId }),
    ];
    const counts = turnCountsOf(rows, context(rows));
    expect(counts.unpushed).toBe(1);
    expect(counts.dirty).toBe(1);
    expect(counts.interrupted).toBe(1);
    expect(counts.total).toBe(4);
  });
});

describe('newestWorktreeObservation', () => {
  it('is null when nothing recorded one, and never the epoch', () => {
    expect(newestWorktreeObservation([row({ id: 1 as ProjectId })])).toBeNull();
    expect(
      newestWorktreeObservation([
        row({ id: 1 as ProjectId, worktreeObservedAt: NOW - 60 }),
        row({ id: 2 as ProjectId, worktreeObservedAt: NOW }),
      ]),
    ).toBe(NOW);
  });
});

// GAP-16b-4: 16b declared `renderTurn` and 16c built the screen; until this host nothing joined
// them, so §10.4a's beat rendered nothing at all.
describe('TurnBeat — the turn seam, filled', () => {
  it('renders TurnScreen with both ways out', () => {
    const rows = [row({ id: 1 as ProjectId, ahead: 2 })];
    render(
      <TurnBeat
        rows={rows}
        queryContext={context(rows)}
        tier="off"
        handlers={{ onShowMe: vi.fn(), onNotNow: vi.fn() }}
        onShowMe={vi.fn()}
      />,
    );
    expect(screen.getByText(SHOW_ME_LABEL)).toBeTruthy();
    expect(screen.getByText(NOT_NOW_LABEL)).toBeTruthy();
  });

  it('hands the rung query to the host and then leaves the beat', () => {
    const rows = [row({ id: 1 as ProjectId, ahead: 2 })];
    const onShowMe = vi.fn();
    const handlers = { onShowMe: vi.fn(), onNotNow: vi.fn() };
    render(
      <TurnBeat
        rows={rows}
        queryContext={context(rows)}
        tier="off"
        handlers={handlers}
        onShowMe={onShowMe}
      />,
    );
    fireEvent.click(screen.getByText(SHOW_ME_LABEL));
    // Rung 1: unpushed. The query is the ladder's, not a string this host restates.
    expect(onShowMe).toHaveBeenCalledWith(TURN_QUERIES[1]);
    expect(handlers.onShowMe).toHaveBeenCalledTimes(1);
  });

  it('takes the ladder down to the unqualified shelf when no rung claims anything', () => {
    const rows = [row({ id: 1 as ProjectId })];
    const onShowMe = vi.fn();
    render(
      <TurnBeat
        rows={rows}
        queryContext={context(rows)}
        tier="off"
        handlers={{ onShowMe: vi.fn(), onNotNow: vi.fn() }}
        onShowMe={onShowMe}
      />,
    );
    fireEvent.click(screen.getByText(SHOW_ME_LABEL));
    // Rung 4's query is the empty string, which is the unfiltered shelf and not a no-op.
    expect(onShowMe).toHaveBeenCalledWith(TURN_QUERIES[4]);
  });

  it('NOT NOW leaves without writing a query', () => {
    const onShowMe = vi.fn();
    const handlers = { onShowMe: vi.fn(), onNotNow: vi.fn() };
    render(
      <TurnBeat
        rows={[]}
        queryContext={context([])}
        tier="off"
        handlers={handlers}
        onShowMe={onShowMe}
      />,
    );
    fireEvent.click(screen.getByText(NOT_NOW_LABEL));
    expect(handlers.onNotNow).toHaveBeenCalledTimes(1);
    expect(onShowMe).not.toHaveBeenCalled();
  });
});

describe('FirstRunHost', () => {
  it('mounts the gate for a library that has never been scanned', async () => {
    mount({ 'roots.suggest': () => [], 'roots.list': () => [] });
    await waitFor(() => {
      expect(document.querySelector('.cdt-fr-view')).not.toBeNull();
    });
    expect(screen.queryByTestId('shelf')).toBeNull();
  });

  it('lets a scanned library through to the shelf beneath', async () => {
    mount(
      { 'roots.suggest': () => [], 'roots.list': () => [] },
      { status: { ...neverScanned, generation: 3, endedAt: NOW }, hasStoredShelf: true },
    );
    await waitFor(() => {
      expect(screen.getByTestId('shelf')).toBeTruthy();
    });
  });

  it('holds blank ground while the core has not answered', () => {
    mount({ 'roots.suggest': () => [], 'roots.list': () => [] }, { status: null });
    // `gateDecision(null, false)` is `'wait'`. Falling through to the shelf here would flash an
    // empty grid under the roots screen.
    expect(screen.queryByTestId('shelf')).toBeNull();
  });

  it('asks the core for its suggestions and its roots, and for nothing else on mount', async () => {
    const fake = fakeAppDeps({ 'roots.suggest': () => [], 'roots.list': () => [root(1, '/a')] });
    render(
      <FirstRunHost
        deps={fake.deps}
        rows={[]}
        firstRunCompletedAt={null}
        status={neverScanned}
        hasStoredShelf={false}
        tier="off"
        onOpenScanSummary={vi.fn()}
        onShowMe={vi.fn()}
      >
        <div data-testid="shelf" />
      </FirstRunHost>,
    );
    await waitFor(() => {
      expect(fake.calls.map((call) => call.name).sort()).toEqual(['roots.list', 'roots.suggest']);
    });
  });
});

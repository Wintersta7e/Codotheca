/**
 * §10's gate, built from the one injected door — and `renderTurn` finally filled.
 *
 * GAP-16b-4: `FirstRunGateDeps.renderTurn` is the seam 16b declared and 16c built the screen
 * for, and until now nothing joined the two. This is that call site, and it is the whole of it:
 * the gate owns *which* beat is mounted and the turn owns its own four-rung ladder.
 */
import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactElement,
  type ReactNode,
} from 'react';

import type {
  ProjectId,
  Reveal,
  Root,
  RootAdd,
  RootSuggestion,
  ScanStatus,
} from '../../generated/protocol.js';
import type { RendererEvent } from '../../shared/channels.js';
import { appearanceFor } from '../art/appearance.js';
import { FirstRunGate, type TurnHandlers } from '../firstrun/FirstRunGate.js';
import type { ScanFeedEvent } from '../firstrun/scanFeed.js';
import { TurnScreen } from '../firstrun/TurnScreen.js';
import type { TurnCounts } from '../firstrun/turn.js';
import type { LanguageCount, RevealDeps, RevealProject } from '../firstrun/revealModel.js';
import type { ResolvedTier } from '../motion/tier.js';
import { attentionCounts } from '../shelf/counts.js';
import type { QueryContext } from '../shelf/evaluate.js';
import type { ShelfRow } from '../shelf/row.js';
import type { AppDeps } from './deps.js';
import { buildQueryContext } from './queryContext.js';

export interface FirstRunHostProps {
  readonly deps: AppDeps;
  readonly rows: readonly ShelfRow[];
  readonly firstRunCompletedAt: number | null;
  readonly status: ScanStatus | null;
  readonly hasStoredShelf: boolean;
  readonly tier: ResolvedTier;
  readonly onOpenScanSummary: () => void;
  /** §10.4a: `SHOW ME` sets the shelf's query, expands every era and resets density. */
  readonly onShowMe: (query: string) => void;
  readonly children: ReactNode;
}

/**
 * §10.3's live feed, from the two topics that carry it. Only the four shapes `ScanFeedEvent`
 * declares are forwarded; a fifth invented here would be a beat the reducer cannot answer.
 */
export function toScanFeedEvent(event: RendererEvent): ScanFeedEvent | null {
  const data = (event.data ?? {}) as Record<string, unknown>;
  if (event.topic === 'projects' && event.event === 'upserted') {
    const row = data['row'] as { id?: unknown; name?: unknown; primaryLanguage?: unknown } | null;
    if (row === null || typeof row.id !== 'number' || typeof row.name !== 'string') return null;
    return {
      kind: 'upserted',
      id: row.id as ProjectId,
      name: row.name,
      primaryLanguage: typeof row.primaryLanguage === 'string' ? row.primaryLanguage : null,
    };
  }
  if (event.topic === 'projects' && event.event === 'merged') {
    return { kind: 'merged', from: data['from'] as ProjectId, into: data['into'] as ProjectId };
  }
  if (event.topic === 'scan' && event.event === 'progress') {
    return {
      kind: 'progress',
      indexedProjects: Number(data['indexedProjects'] ?? 0),
      walkedDirs: Number(data['walkedDirs'] ?? 0),
      foundRepos: Number(data['foundRepos'] ?? 0),
    };
  }
  if (event.topic === 'scan' && (event.event === 'finished' || event.event === 'cancelled')) {
    return { kind: 'finished' };
  }
  return null;
}

/** §10.1a's `<root> · <root> · N ROOTS`, from the rows the core actually holds. */
export function rootLineOf(roots: readonly Root[]): string {
  const enabled = roots.filter((root) => root.enabled);
  if (enabled.length === 0) return '';
  const noun = enabled.length === 1 ? 'ROOT' : 'ROOTS';
  return `${enabled.map((root) => root.pathDisplay).join(' · ')} · ${String(enabled.length)} ${noun}`;
}

/** §10.4a's four rungs, counted with the shelf's own predicates rather than a second set. */
export function turnCountsOf(rows: readonly ShelfRow[], ctx: QueryContext): TurnCounts {
  const counts = attentionCounts(rows, ctx);
  return {
    unpushed: counts['unpushed'] ?? 0,
    dirty: counts['uncommitted'] ?? 0,
    // §8.0b has no interrupted chip, so this one predicate is counted here. It is the flag the
    // row already carries, not a second reading of git.
    interrupted: rows.filter((row) => row.interruptedOp !== null).length,
    total: counts['all'] ?? 0,
  };
}

function languageTally(rows: readonly ShelfRow[]): readonly LanguageCount[] {
  const tally = new Map<string, number>();
  for (const row of rows) {
    if (row.primaryLanguage === null) continue;
    tally.set(row.primaryLanguage, (tally.get(row.primaryLanguage) ?? 0) + 1);
  }
  return [...tally].map(([name, count]) => ({ name, count })).sort((a, b) => b.count - a.count);
}

export interface TurnBeatProps {
  readonly rows: readonly ShelfRow[];
  readonly queryContext: QueryContext;
  readonly tier: ResolvedTier;
  readonly handlers: TurnHandlers;
  readonly onShowMe: (query: string) => void;
}

/**
 * §10.4a's beat, and the reason it is a component rather than an inline element: the gate takes
 * `renderTurn` as a callback, and a callback is not something a test can mount. This is the seam
 * GAP-16b-4 named, filled and reachable.
 */
export function TurnBeat(props: TurnBeatProps): ReactElement {
  const { rows, handlers, onShowMe } = props;
  return (
    <TurnScreen
      counts={turnCountsOf(rows, props.queryContext)}
      worktreeObservedAt={newestWorktreeObservation(rows)}
      tier={props.tier}
      onShowMe={(query) => {
        // §10.4a: the query, the expand and the density reset are shelf state, which the turn is
        // explicit about not owning. The host writes them and then leaves the beat.
        onShowMe(query);
        handlers.onShowMe();
      }}
      onNotNow={handlers.onNotNow}
    />
  );
}

/** `null` when no project recorded one — never `0`, which would date the claim to the epoch. */
export function newestWorktreeObservation(rows: readonly ShelfRow[]): number | null {
  let newest: number | null = null;
  for (const row of rows) {
    if (row.worktreeObservedAt === null) continue;
    newest = newest === null ? row.worktreeObservedAt : Math.max(newest, row.worktreeObservedAt);
  }
  return newest;
}

export function FirstRunHost(props: FirstRunHostProps): ReactElement {
  const { deps, rows, tier, onOpenScanSummary, onShowMe } = props;
  const { request, subscribe, pickRoot, nowMs, now } = deps;
  // The same builder the shelf uses, so the turn's rungs cannot disagree with the grid's chips.
  const queryContext = useMemo(
    () => buildQueryContext({ rows, now: now(), firstRunCompletedAt: props.firstRunCompletedAt }),
    [rows, now, props.firstRunCompletedAt],
  );
  const [roots, setRoots] = useState<readonly Root[]>([]);

  useEffect(() => {
    let live = true;
    void request('roots.list', {}).then(
      (answer) => {
        if (live) setRoots(answer);
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [request]);

  const rowsById = useMemo(() => new Map(rows.map((row) => [row.id, row] as const)), [rows]);

  const revealDeps = useMemo<RevealDeps>(
    () => ({
      nowSecs: now(),
      languageTally: languageTally(rows),
      // `null` is *not classified yet*. A zero here would claim a finished pass found none.
      referenceCount: rows.some((row) => row.authoredByUser !== null)
        ? rows.filter((row) => row.isReference).length
        : null,
      project: (id: ProjectId): RevealProject | null => {
        const row = rowsById.get(id);
        return row === undefined
          ? null
          : {
              name: row.name,
              birthYear: row.birthYear,
              primaryLanguage: row.primaryLanguage,
            };
      },
    }),
    [now, rows, rowsById],
  );

  const renderTurn = useCallback(
    (handlers: TurnHandlers): ReactNode => (
      <TurnBeat
        rows={rows}
        queryContext={queryContext}
        tier={tier}
        handlers={handlers}
        onShowMe={onShowMe}
      />
    ),
    [rows, queryContext, tier, onShowMe],
  );

  const gateDeps = useMemo(
    () => ({
      nowMs,
      suggestRoots: (): Promise<readonly RootSuggestion[]> => request('roots.suggest', {}),
      /**
       * GAP-16b-1, still open and **not filled here**. `roots.add` is privileged and takes
       * `pathBytes`; §1.3 bars `RootSuggestion` from carrying bytes and §2.4 bars the renderer
       * from originating a path, so committing a ticked suggestion needs a shell channel that
       * resolves the display string against the core's own list — plan 16's `SuggestionCache`,
       * which does not exist. Owner: plan 16 plus `protocol/schema/protocol.json`.
       *
       * It **resolves** rather than rejects on purpose: the gate awaits this inside `onDig`, so
       * a rejection would leave `DIG IN` spinning for the rest of the session. A resolved
       * `RootAdd` with no root added lets the beat advance and the scan report what it found,
       * which is nothing under a root nobody added — visible, rather than a hung button.
       */
      commitSuggestion: (): Promise<RootAdd> =>
        Promise.resolve({ root: null, refusedBecause: null, estimatedDirs: null }),
      pickRoot,
      startScan: async (): Promise<void> => {
        await request('scan.start', { full: true });
      },
      loadReveal: (): Promise<Reveal> => request('stats.reveal', {}),
      revealDeps,
      scan: {
        // §10.3's stripe. The seed is the tile's own name, which is what the card uses before a
        // `seedBasename` exists for it.
        jewelFor: (tile: { name: string; primaryLanguage: string | null }): string | null =>
          appearanceFor({ seedBasename: tile.name, rerollOffset: 0 }, 0, tile.primaryLanguage)
            .jewel,
        onSkipAhead: (): void => undefined,
        onOpenScanSummary,
      },
      subscribe: (cb: (event: ScanFeedEvent) => void): (() => void) =>
        subscribe((event) => {
          const feed = toScanFeedEvent(event);
          if (feed !== null) cb(feed);
        }),
      renderTurn,
      rootLine: rootLineOf(roots),
    }),
    [nowMs, request, pickRoot, revealDeps, onOpenScanSummary, subscribe, renderTurn, roots],
  );

  return (
    <FirstRunGate
      deps={gateDeps}
      status={props.status}
      hasStoredShelf={props.hasStoredShelf}
      tier={tier}
    >
      {props.children}
    </FirstRunGate>
  );
}

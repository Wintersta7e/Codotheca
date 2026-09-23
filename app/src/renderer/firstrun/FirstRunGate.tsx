import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import type { ReactElement, ReactNode } from 'react';
import {
  ARRIVAL_BATCH_MS,
  INITIAL_FIRST_RUN,
  SETTLE_HOLD_MS,
  firstRunReducer,
  shouldRunFirstRun,
} from './phase';
import { INITIAL_SCAN_FEED, milestoneCrossed, scanFeedReducer } from './scanFeed';
import { MILESTONE_FIGURES, panelFor } from './revealModel';
import { initialTicks, refusedRow, toRow } from './rootRows';
import { RootsScreen } from './RootsScreen';
import { ScanScreen } from './ScanScreen';
import { RevealScreen } from './RevealScreen';
import type { PendingConfirm } from './RootsScreen';
import type { ScanScreenDeps } from './ScanScreen';
import type { RevealDeps, RevealPanel } from './revealModel';
import type { ScanFeedEvent } from './scanFeed';
import type { RootRow } from './rootRows';
import type { Reveal, RootAdd, RootSuggestion, ScanStatus } from '../../generated/protocol';
import type { PickRootReply } from '../../shared/channels';
import type { ResolvedTier } from '../motion/tier';

export interface TurnHandlers {
  readonly onShowMe: () => void;
  readonly onNotNow: () => void;
}

export interface FirstRunGateDeps {
  /** R3: monotonic milliseconds, handed in. Nothing here calls `Date.now()`. */
  readonly nowMs: () => number;
  readonly suggestRoots: () => Promise<readonly RootSuggestion[]>;
  /**
   * GAP-16b-1: no command yet commits a suggestion the core produced. The shell resolves this
   * through plan 16's `SuggestionCache`; the argument is the display string, never a path the
   * renderer built.
   */
  readonly commitSuggestion: (pathDisplay: string) => Promise<RootAdd>;
  /**
   * The native dialog behind `IPC_PICK_ROOT`, exactly as `CodothecaBridge.pickRoot` answers it.
   * The three variants are kept apart rather than collapsed to `RootAdd | null`: a dialog the
   * user cancelled and a dialog that failed are different events, and an adapter that erased the
   * difference would let a `failed` reply be drawn as a folder the user picked.
   */
  readonly pickRoot: (confirmLarge: boolean) => Promise<PickRootReply>;
  readonly startScan: () => Promise<void>;
  readonly loadReveal: () => Promise<Reveal>;
  readonly revealDeps: RevealDeps;
  readonly scan: ScanScreenDeps;
  readonly subscribe: (cb: (event: ScanFeedEvent) => void) => () => void;
  /**
   * GAP-16b-4: §10.4a's turn is plan 16c's `TurnScreen`, which does not exist yet. The gate owns
   * *which* beat is mounted and the turn owns its own four-rung ladder, so the beat is handed in
   * rather than duplicated here — 16c wires `<TurnScreen …/>` into this one call site. It has no
   * production implementation until then, and the gate cannot invent one without re-deriving the
   * ladder this plan does not own.
   */
  readonly renderTurn: (handlers: TurnHandlers) => ReactNode;
  readonly rootLine: string;
}

export interface FirstRunGateProps {
  readonly deps: FirstRunGateDeps;
  /** `null` until the core's snapshot arrives. */
  readonly status: ScanStatus | null;
  /** True when the stored snapshot painted rows — proof a scan has already run. */
  readonly hasStoredShelf: boolean;
  readonly tier: ResolvedTier;
  readonly children: ReactNode;
}

/**
 * §11.2 paints from the stored snapshot before the core joins, and criterion 15 times process
 * start → first tile pixel. A stored shelf can only exist after a scan, so it is proof first
 * run is over and the gate lets it through without waiting. With nothing to paint there is no
 * tile to time, so holding the ground costs nothing and avoids flashing the shelf under the
 * roots screen.
 */
export function gateDecision(
  status: ScanStatus | null,
  hasStoredShelf: boolean,
): 'shelf' | 'first-run' | 'wait' {
  if (hasStoredShelf) return 'shelf';
  if (status === null) return 'wait';
  return shouldRunFirstRun(status) ? 'first-run' : 'shelf';
}

export function FirstRunGate(props: FirstRunGateProps): ReactElement {
  const { deps, status, hasStoredShelf, tier, children } = props;
  /**
   * §10 is a **sequence**, so its gate is an entry condition and not a live one.
   *
   * `gateDecision` reads `scan_run.id` and a painted shelf, and **`DIG` creates both**: the
   * `run_started` event gives the status a `runId` within milliseconds, and the walk it starts
   * fills the projection. Re-deciding on every render therefore ended first run **from inside
   * first run** — measured against the real app, the scanning screen, the reveal and the turn
   * were all gone within a second of `DIG`, replaced by an empty shelf, and §10's entire beat
   * sequence had never once run in the product.
   *
   * Once the gate has entered, the reducer owns the screen until it reaches `shelf`, which is
   * what §10.5's *the reveal never replays* is really about: a run that exists is proof first
   * run is **over**, except while it is the run first run just started.
   */
  const entry = gateDecision(status, hasStoredShelf);
  const entered = useRef(false);
  if (entry === 'first-run') entered.current = true;

  const [state, dispatch] = useReducer(firstRunReducer, INITIAL_FIRST_RUN);
  const decision = entered.current && state.phase !== 'shelf' ? 'first-run' : entry;
  const [feed, feedDispatch] = useReducer(scanFeedReducer, INITIAL_SCAN_FEED);
  const [suggestions, setSuggestions] = useState<readonly RootSuggestion[]>([]);
  const [extra, setExtra] = useState<readonly RootRow[]>([]);
  const [ticked, setTicked] = useState<ReadonlySet<string>>(new Set());
  const [pendingConfirm, setPendingConfirm] = useState<PendingConfirm | null>(null);
  const [busy, setBusy] = useState(false);
  const [reveal, setReveal] = useState<Reveal | null>(null);
  const [milestone, setMilestone] = useState<RevealPanel | null>(null);
  const foundRef = useRef(0);

  const { nowMs, suggestRoots, commitSuggestion, pickRoot, startScan, loadReveal, subscribe } =
    deps;
  const { revealDeps } = deps;
  const active = decision === 'first-run' && state.phase !== 'shelf';

  // §10.1a: three small named files, read by the core. The renderer asks once.
  useEffect(() => {
    if (decision !== 'first-run') return undefined;
    let live = true;
    void suggestRoots().then((rows) => {
      if (!live) return;
      setSuggestions(rows);
      setTicked(initialTicks(rows));
    });
    return () => {
      live = false;
    };
  }, [decision, suggestRoots]);

  // The live feed. One subscription, released when the beats are done. It opens on the roots
  // screen, before `DIG` starts anything: the core answers `scan.start` and announces the walk as
  // separate messages, so a small walk can finish before the phase leaves `roots`, and a
  // `finished` that reached no listener left the scan screen waiting for ever.
  useEffect(() => {
    if (!active) return undefined;
    return subscribe(feedDispatch);
  }, [active, subscribe]);

  // §10.3: a fixed cadence, so a batch is whatever the walk produced in the last 600 ms and the
  // acceleration is emergent rather than fitted to a total that does not exist.
  useEffect(() => {
    if (state.phase !== 'scanning') return undefined;
    const id = setInterval(() => {
      feedDispatch({ kind: 'flush' });
    }, ARRIVAL_BATCH_MS);
    return () => {
      clearInterval(id);
    };
  }, [state.phase]);

  // §10.3a: one settle at walk completion, then 700 ms before the reveal takes the screen. The
  // tick carries the real clock reading, so the reducer's own hold guard is what decides — a
  // tick that pre-added the hold would make that guard inert.
  useEffect(() => {
    if (state.phase !== 'scanning' || !feed.finished) return undefined;
    dispatch({ kind: 'walk_finished', at: nowMs() });
    const id = setTimeout(() => {
      dispatch({ kind: 'tick', at: nowMs() });
    }, SETTLE_HOLD_MS);
    return () => {
      clearTimeout(id);
    };
  }, [state.phase, feed.finished, nowMs]);

  // §10.3a: a milestone renders one reveal figure computed at that instant. It comes from the
  // same call as the reveal, so the two cannot disagree.
  useEffect(() => {
    const crossed = milestoneCrossed(foundRef.current, feed.found);
    foundRef.current = feed.found;
    if (crossed === null || state.phase !== 'scanning') return;
    const key = MILESTONE_FIGURES[crossed];
    if (key === undefined) return;
    void loadReveal().then((r) => {
      setMilestone(panelFor(key, r, revealDeps));
    });
  }, [feed.found, state.phase, loadReveal, revealDeps]);

  // §10.4: the reveal fires at walk completion, not at full history — otherwise it waits
  // minutes. A shelf with no projects never reaches it and gets §11.1 instead.
  useEffect(() => {
    if (state.phase !== 'reveal' || reveal !== null) return;
    void loadReveal().then((r) => {
      if ((r.projectCount.value ?? 0) <= 0) dispatch({ kind: 'nothing_found' });
      else setReveal(r);
    });
  }, [state.phase, reveal, loadReveal]);

  const rows = useMemo<readonly RootRow[]>(
    () => [...suggestions.map((s) => toRow(s, ticked.has(s.pathDisplay))), ...extra],
    [suggestions, ticked, extra],
  );

  const absorb = useCallback((add: RootAdd, pathDisplay: string) => {
    setExtra((current) => [...current, refusedRow(add, pathDisplay)]);
    if (add.refusedBecause === 'too_many_directories' && add.estimatedDirs !== null) {
      setPendingConfirm({ pathDisplay, estimatedDirs: add.estimatedDirs });
    }
  }, []);

  const rootsDeps = useMemo(
    () => ({
      onToggleRoot: (key: string) => {
        setTicked((current) => {
          const next = new Set(current);
          if (!next.delete(key)) next.add(key);
          return next;
        });
      },
      onConsent: (granted: boolean) => {
        dispatch({ kind: 'consent', granted });
      },
      // §2.4: a folder can only reach the core through a dialog the shell owns. `cancelled` adds
      // nothing by design. GAP-16b-5: `failed` carries a `BridgeError` this screen has nowhere to
      // draw — §10.1b specifies no failure state for the dialog, and 17c's error surfaces explain
      // a *project's* error kind, not a shell call. The reply is left whole at this call site
      // rather than erased in an adapter, so the surface can be wired in one place later.
      onAddFolder: () => {
        void pickRoot(false).then((reply) => {
          if (reply.kind === 'added') absorb(reply.add, reply.add.root?.pathDisplay ?? '');
        });
      },
      onConfirmLarge: (pathDisplay: string) => {
        setPendingConfirm(null);
        void pickRoot(true).then((reply) => {
          if (reply.kind === 'added') absorb(reply.add, pathDisplay);
        });
      },
      onDig: () => {
        setBusy(true);
        // Commit before start, or the walk has no roots.
        void (async () => {
          for (const key of ticked) await commitSuggestion(key);
          await startScan();
          setBusy(false);
          dispatch({ kind: 'dig' });
        })();
      },
    }),
    [pickRoot, commitSuggestion, startScan, ticked, absorb],
  );

  const turnHandlers = useMemo<TurnHandlers>(
    () => ({
      onShowMe: () => {
        dispatch({ kind: 'show_me' });
      },
      onNotNow: () => {
        dispatch({ kind: 'not_now' });
      },
    }),
    [],
  );

  if (decision === 'wait') return <div className="cdt-fr-view" data-effects-tier={tier} />;
  if (!active) return <>{children}</>;

  switch (state.phase) {
    case 'roots':
      return (
        <RootsScreen
          deps={rootsDeps}
          rows={rows}
          ticked={ticked}
          consented={state.consented}
          pendingConfirm={pendingConfirm}
          tier={tier}
          busy={busy}
        />
      );
    case 'scanning':
      return (
        <ScanScreen
          deps={{
            ...deps.scan,
            onSkipAhead: () => {
              dispatch({ kind: 'skip_ahead' });
            },
          }}
          feed={feed}
          rootLine={deps.rootLine}
          milestone={milestone}
          tier={tier}
        />
      );
    case 'reveal':
      return reveal === null ? (
        <div className="cdt-fr-view" data-effects-tier={tier} />
      ) : (
        <RevealScreen
          reveal={reveal}
          deps={revealDeps}
          tier={tier}
          onGoOn={() => {
            dispatch({ kind: 'go_on' });
          }}
        />
      );
    case 'turn':
      // Plan 16c. The gate owns which beat is mounted; the turn owns its own ladder.
      return <>{deps.renderTurn(turnHandlers)}</>;
    default:
      return <>{children}</>;
  }
}

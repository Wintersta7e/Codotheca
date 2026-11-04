/**
 * §43.2's status band — block 1a, the first block in the shelf's one scroll container, above
 * §8.0's notice slot. It draws the level, the rings, the live session and the week's rhythm from
 * the one `MotivationState` the window holds, and draws no unknown as a zero.
 *
 * Nothing in it moves at rest: a ring, the bar and a cell change by a cut at every tier, and its
 * one motion, §43.6's flip, follows the user's input. Nothing in it is a live region. Every
 * element quiet mode removes carries `data-motivation`.
 */
import { type CSSProperties, type ReactElement, type RefObject, useEffect, useRef } from 'react';
import type { MotivationState, ProjectId, ProjectRow, SessionRef } from '../../generated/protocol';
import { DATA_MOTIVATION } from '../../shared/motivationQuiet';
import type { AppDeps } from '../app/deps';
import type { AppRoute, ProfileAnchor } from '../app/route';
import { BENCH_LABEL, useBenchElapsed } from '../card/useBenchElapsed';
import { useWindowActive } from '../motion/useWindowActive';
import { useSettledDividers } from './bandDividers';
import { LEVEL_LABEL, RHYTHM_LABEL } from './copy';
import { levelBlockModel } from './levelBlock';
import { MotivationLineButton } from './MotivationLineButton';
import { rhythmModel } from './rhythm';
import { ringsModel, type RingGlyph } from './rings';
import { useBandOnScreen } from './useBandOnScreen';
import { useLevelFlip } from './useLevelFlip';

/**
 * §43.2: the shelf is the mounted screen, the layer is shown, and the library holds a live row.
 * An empty library has nothing the layer measures, and a band there would stamp day one on a
 * shelf with nothing on it.
 */
export function bandRenders(input: {
  readonly route: AppRoute['kind'];
  readonly state: MotivationState | null;
  readonly rows: readonly ProjectRow[];
}): boolean {
  return (
    input.route === 'shelf' &&
    input.state !== null &&
    input.state.shown &&
    input.rows.some((row) => !row.isHidden)
  );
}

/** The session the band mirrors: the most recently started one still open. */
export interface BandSession {
  readonly projectId: ProjectId;
  readonly name: string;
  readonly startedAt: number;
}

export function benchSession(
  sessions: ReadonlyMap<ProjectId, SessionRef>,
  rows: readonly ProjectRow[],
): BandSession | null {
  let latest: SessionRef | null = null;
  for (const session of sessions.values()) {
    if (session.endedAt !== null) continue;
    if (latest === null || session.startedAt > latest.startedAt) latest = session;
  }
  if (latest === null) return null;
  const projectId = latest.projectId;
  const row = rows.find((candidate) => candidate.id === projectId);
  // A session on a row this window has not read has no name to mirror, so nothing is drawn.
  if (row === undefined) return null;
  return { projectId, name: row.name, startedAt: latest.startedAt };
}

export interface StatusBandProps {
  readonly route: AppRoute['kind'];
  readonly state: MotivationState | null;
  readonly rows: readonly ProjectRow[];
  readonly sessions: ReadonlyMap<ProjectId, SessionRef>;
  /** Unix seconds, read when called — the session block's elapsed is re-derived on each paint. */
  readonly now: () => number;
  readonly onOpenProject: (id: ProjectId) => void;
  /** §43.13: the number and a level line open Profile at the level, a badge line at its badge. */
  readonly onOpenProfile: (at: ProfileAnchor) => void;
  /** The shelf's one scroll container, the root the band's visibility is measured against. */
  readonly scrollRef: RefObject<HTMLElement | null>;
  readonly request: AppDeps['request'];
  /**
   * Each observed crossing of the scroll container, and `null` while the band is not observed —
   * §43.2's condensed block is drawn only on a reported `false`. A stable function: a new one
   * re-subscribes the observer.
   */
  readonly onPlacement?: (onScreen: boolean | null) => void;
}

export function StatusBand(props: StatusBandProps): ReactElement | null {
  const { state } = props;
  if (state === null || !bandRenders({ route: props.route, state, rows: props.rows })) {
    return null;
  }
  // Mounted only while it renders, so the observer starts with a band to observe.
  return <Band {...props} state={state} />;
}

const quiet = (id: string): Record<string, string> => ({ [DATA_MOTIVATION]: id });

/** A custom property the stylesheet reads; a fraction is a number, never a width string. */
const fractionStyle = (name: string, value: number): CSSProperties => ({ [name]: String(value) });

function Ring({ ring }: { readonly ring: RingGlyph }): ReactElement {
  return (
    <div className="cdt-band-ring" role="img" aria-label={ring.name}>
      <div
        className="cdt-band-ring-glyph"
        data-fill={ring.fill}
        style={fractionStyle('--cdt-band-arc', ring.fraction)}
      >
        <span
          className="cdt-band-ring-centre"
          data-closed={ring.closed ? 'true' : undefined}
          data-unknown={ring.unknown ? 'true' : undefined}
        >
          {ring.centre}
        </span>
      </div>
      <div className="cdt-band-ring-text">
        <span className="cdt-band-ring-label">{ring.label}</span>
        {ring.sub === null ? null : <span className="cdt-band-ring-sub">{ring.sub}</span>}
      </div>
    </div>
  );
}

function Band(props: StatusBandProps & { readonly state: MotivationState }): ReactElement {
  const { state, scrollRef, request, onOpenProject, onOpenProfile } = props;
  const bandRef = useRef<HTMLDivElement>(null);
  const onScreen = useBandOnScreen(bandRef, scrollRef, props.onPlacement);
  useSettledDividers(bandRef);
  const active = useWindowActive();
  const shown = onScreen && active;
  const wasShown = useRef(false);
  // §43.6: the flip listens on the band's place alone, since a window gaining focus fires it.
  const flip = useLevelFlip(state, onScreen);
  const playing = flip.playing;

  // §43.9: each transition into *on screen in an active window* tells the core the band was
  // seen. A hidden or unfocused window sends nothing, and staying shown sends nothing more.
  useEffect(() => {
    if (shown && !wasShown.current) {
      void request('motivation.acknowledge', { event: 'bandShown', level: null }).then(
        () => undefined,
        () => undefined,
      );
    }
    wasShown.current = shown;
  }, [shown, request]);

  const session = benchSession(props.sessions, props.rows);
  const elapsed = useBenchElapsed(session?.startedAt ?? null, props.now);

  const level = levelBlockModel(state.level, state.xpBar);
  const rings = state.rings === null ? null : ringsModel(state.rings);
  const rhythm = state.rings === null ? null : rhythmModel(state.rings.rhythm);
  const focusArc =
    rings === null || rings.waived
      ? 0
      : (rings.rings.find((r) => r.key === 'focus')?.fraction ?? 0);

  return (
    <div className="cdt-band" ref={bandRef}>
      <div className="cdt-band-level" {...quiet('level')}>
        <button
          type="button"
          className="cdt-band-level-number"
          aria-label={level.name}
          onClick={() => {
            // It replays nothing: the number opens Profile, and that is all a click on it does.
            onOpenProfile({ section: 'level' });
          }}
        >
          <span className="cdt-band-level-figure" data-unknown={level.unknown ? 'true' : undefined}>
            {playing === null ? null : (
              // The leaving number is motion, not content: the button's name is the level now.
              <span className="cdt-level-digit" data-flip="old" aria-hidden="true">
                {String(playing.from)}
              </span>
            )}
            <span
              className="cdt-level-digit"
              data-flip={playing === null ? undefined : 'new'}
              ref={flip.incoming}
            >
              {level.number}
            </span>
          </span>
          <span className="cdt-band-level-label">{LEVEL_LABEL}</span>
        </button>
        <div className="cdt-band-xp" {...quiet('xp')}>
          <span className="cdt-band-xp-line">{level.line}</span>
          {level.bar === null ? null : (
            <div className="cdt-band-xp-track">
              <div
                className="cdt-band-xp-fill cdt-xp-fill"
                data-flip={playing === null ? undefined : 'surge'}
                style={fractionStyle('--cdt-band-fill', level.bar)}
              />
              {playing === null ? null : (
                <div
                  className="cdt-band-xp-flash cdt-xp-fill"
                  data-flip="flash"
                  aria-hidden="true"
                />
              )}
            </div>
          )}
        </div>
        {state.pendingLine === null ? null : (
          <MotivationLineButton
            line={state.pendingLine}
            level={state.level}
            onScreen={shown}
            request={request}
            onOpenProfile={onOpenProfile}
          />
        )}
      </div>

      {rings === null ? null : (
        <>
          <div className="cdt-band-divider" aria-hidden="true" />
          <div className="cdt-band-rings" {...quiet('rings')}>
            {rings.waived ? (
              <div className="cdt-band-waived">
                <div className="cdt-band-waived-labels">
                  {rings.labels.map((label) => (
                    <span key={label} className="cdt-band-ring-label">
                      {label}
                    </span>
                  ))}
                </div>
                <span className="cdt-band-ring-sub">{rings.sentence}</span>
              </div>
            ) : (
              rings.rings.map((ring) => <Ring key={ring.key} ring={ring} />)
            )}
          </div>
        </>
      )}

      {session === null || elapsed === null ? null : (
        <>
          <div className="cdt-band-divider" aria-hidden="true" />
          <button
            type="button"
            className="cdt-band-session"
            onClick={() => {
              onOpenProject(session.projectId);
            }}
          >
            <span
              className="cdt-band-session-glyph"
              aria-hidden="true"
              style={fractionStyle('--cdt-band-arc', focusArc)}
            />
            <span className="cdt-band-session-text">
              <span className="cdt-band-session-name">{session.name}</span>
              <span className="cdt-band-session-bench">{`${BENCH_LABEL} · ${elapsed}`}</span>
            </span>
          </button>
        </>
      )}

      <div className="cdt-band-spacer" />

      {rhythm === null ? null : (
        <div className="cdt-band-rhythm" {...quiet('rhythm')}>
          <div className="cdt-band-rhythm-head">
            <span className="cdt-band-rhythm-label">{RHYTHM_LABEL}</span>
            <span className="cdt-band-rhythm-line">{rhythm.line}</span>
            {rhythm.partial === null ? null : (
              <span className="cdt-band-rhythm-line">{rhythm.partial}</span>
            )}
          </div>
          <div className="cdt-band-week">
            {rhythm.cells.map((cell) => (
              <div key={cell.name} className="cdt-band-day" role="img" aria-label={cell.name}>
                <div className="cdt-band-cell" data-day={cell.day} />
                <span className="cdt-band-day-letter">{cell.letter}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

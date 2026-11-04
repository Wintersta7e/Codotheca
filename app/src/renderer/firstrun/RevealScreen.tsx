import { useEffect, useRef, useState } from 'react';
import type { ReactElement } from 'react';
import {
  CLOSE_LABEL,
  EVIDENCE_FOOTER,
  EVIDENCE_HEADING,
  GO_ON_LABEL,
  REVEAL_EYEBROW,
  SHOW_WORKING_LABEL,
  UNCOMPUTED_NOTE,
  revealHeadline,
} from './copy';
import { revealPanels, spanYears } from './revealModel';
import type { RevealDeps } from './revealModel';
import { badgePanel, levelPanel, revealGridFloorPx, revealSequence } from './revealMotivation';
import type {
  BadgeKind,
  BadgesListed,
  MotivationAck,
  MotivationState,
  Reveal,
} from '../../generated/protocol';
import { DATA_MOTIVATION } from '../../shared/motivationQuiet';
import type { ResolvedTier } from '../motion/tier';
import { useWindowActive } from '../motion/useWindowActive';

/** §43.7: what the reveal's `LEVEL` and `BADGE EARNED` read, and the two writes they make. */
export interface RevealMotivation {
  /** The one motivation read; `null` is *not read*, and draws neither panel. */
  readonly state: MotivationState | null;
  /** §40's `badges.list`; `null` before it answers or when it is refused. */
  readonly badges: BadgesListed | null;
  readonly acknowledge: (event: MotivationAck, level: number | null) => Promise<void>;
  readonly markLanded: (kind: BadgeKind) => Promise<void>;
}

export interface RevealScreenProps {
  readonly reveal: Reveal;
  readonly deps: RevealDeps;
  readonly motivation: RevealMotivation;
  readonly tier: ResolvedTier;
  readonly onGoOn: () => void;
}

export function RevealScreen(props: RevealScreenProps): ReactElement {
  const { reveal, deps, motivation, tier, onGoOn } = props;
  const [open, setOpen] = useState<string | null>(null);
  const shown = motivation.state?.shown === true;
  const level = motivation.state === null ? null : levelPanel(motivation.state);
  const badge = badgePanel(motivation.badges, shown);
  const panels = revealSequence(revealPanels(reveal, deps), level, badge);
  const basis = reveal.spanDays.basis;
  const complete = basis.projectsCovered >= basis.projectsTotal && basis.historyComplete;

  // §43.7: a panel displayed in an active window records what the user saw. LEVEL sends the
  // value it shows — `null` when not computed — once per value, so a refresh in place re-sends;
  // BADGE EARNED lands its award once. A hidden or unfocused window sends nothing.
  const active = useWindowActive();
  const { acknowledge, markLanded } = motivation;
  const levelDrawn = level !== null;
  const shownLevel = level?.shownLevel ?? null;
  const sentLevel = useRef<string | null>(null);
  useEffect(() => {
    if (!active || !levelDrawn) return;
    const key = shownLevel === null ? 'none' : String(shownLevel);
    if (sentLevel.current === key) return;
    sentLevel.current = key;
    acknowledge('revealLevelShown', shownLevel).then(
      () => undefined,
      // A refused write is asked again the next time the panel is displayed.
      () => {
        if (sentLevel.current === key) sentLevel.current = null;
      },
    );
  }, [active, levelDrawn, shownLevel, acknowledge]);

  const toLand = badge?.toLand ?? null;
  const landing = useRef<BadgeKind | null>(null);
  useEffect(() => {
    if (!active || toLand === null || landing.current === toLand) return;
    landing.current = toLand;
    markLanded(toLand).then(
      () => undefined,
      () => {
        if (landing.current === toLand) landing.current = null;
      },
    );
  }, [active, toLand, markLanded]);

  const floor = revealGridFloorPx(panels.length);

  return (
    <div className="cdt-fr-view cdt-fr-view--reveal" data-effects-tier={tier}>
      <div className="cdt-fr-column cdt-fr-column--reveal">
        {/* One number from one call: the eyebrow and the PROJECTS panel cannot disagree. */}
        <span className="cdt-fr-eyebrow">{REVEAL_EYEBROW(reveal.projectCount.value ?? 0)}</span>
        <h1 className="cdt-fr-reveal-headline">
          {revealHeadline(spanYears(reveal.spanDays.value), complete)}
        </h1>

        <div className={`cdt-fr-panels${floor === 200 ? ' cdt-fr-panels--dense' : ''}`}>
          {panels.map((panel) => {
            const isOpen = open === panel.key;
            const edge = panel.edge === null ? '' : ` cdt-fr-panel--${panel.edge}`;
            return (
              <button
                type="button"
                key={panel.key}
                data-testid="fr-panel"
                className={`cdt-fr-panel${edge}`}
                style={{ animationDelay: `${String(panel.delaySec)}s` }}
                aria-expanded={isOpen}
                {...(panel.quiet === null ? {} : { [DATA_MOTIVATION]: panel.quiet })}
                onClick={() => {
                  setOpen(isOpen ? null : panel.key);
                }}
              >
                <span className="cdt-fr-panel-head">
                  <span className="cdt-fr-panel-key">{panel.label}</span>
                  <span className="cdt-fr-panel-working">
                    {isOpen ? CLOSE_LABEL : SHOW_WORKING_LABEL}
                  </span>
                </span>

                {panel.value === null ? (
                  // Never render unknown as zero: the figure says it is not computed.
                  <span
                    className="cdt-fr-panel-value cdt-fr-panel-value--uncomputed"
                    data-testid="fr-panel-value"
                  >
                    {UNCOMPUTED_NOTE}
                  </span>
                ) : (
                  <span
                    className={`cdt-fr-panel-value${panel.wide ? ' cdt-fr-panel-value--name' : ''}`}
                    data-testid="fr-panel-value"
                  >
                    {panel.value}
                  </span>
                )}

                {panel.coverage === null ? null : (
                  <span className="cdt-fr-panel-coverage" data-testid="fr-panel-coverage">
                    {panel.coverage}
                  </span>
                )}

                {panel.caption === '' ? null : (
                  <span className="cdt-fr-panel-caption">{panel.caption}</span>
                )}

                {isOpen ? (
                  <span className="cdt-fr-evidence">
                    <span className="cdt-fr-evidence-label">{EVIDENCE_HEADING}</span>
                    <span className="cdt-fr-evidence-body" data-testid="fr-evidence-body">
                      {panel.evidence}
                    </span>
                  </span>
                ) : null}
              </button>
            );
          })}
        </div>

        <div className="cdt-fr-commit">
          <button type="button" className="cdt-fr-primary" onClick={onGoOn}>
            {GO_ON_LABEL}
          </button>
          {/* Ornament: it repeats what the raised SHOW WORKING label now says on every panel. */}
          <span className="cdt-fr-footer">{EVIDENCE_FOOTER}</span>
        </div>
      </div>
    </div>
  );
}

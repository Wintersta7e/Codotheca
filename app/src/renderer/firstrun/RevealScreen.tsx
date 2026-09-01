import { useState } from 'react';
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
import type { RevealDeps, RevealKey } from './revealModel';
import type { Reveal } from '../../generated/protocol';
import type { ResolvedTier } from '../motion/tier';

export interface RevealScreenProps {
  readonly reveal: Reveal;
  readonly deps: RevealDeps;
  readonly tier: ResolvedTier;
  readonly onGoOn: () => void;
}

export function RevealScreen(props: RevealScreenProps): ReactElement {
  const { reveal, deps, tier, onGoOn } = props;
  const [open, setOpen] = useState<RevealKey | null>(null);
  const panels = revealPanels(reveal, deps);
  const basis = reveal.spanDays.basis;
  const complete = basis.projectsCovered >= basis.projectsTotal && basis.historyComplete;

  return (
    <div className="cdt-fr-view cdt-fr-view--reveal" data-effects-tier={tier}>
      <div className="cdt-fr-column cdt-fr-column--reveal">
        {/* One number from one call: the eyebrow and the PROJECTS panel cannot disagree. */}
        <span className="cdt-fr-eyebrow">{REVEAL_EYEBROW(reveal.projectCount.value ?? 0)}</span>
        <h1 className="cdt-fr-reveal-headline">
          {revealHeadline(spanYears(reveal.spanDays.value), complete)}
        </h1>

        <div className="cdt-fr-panels">
          {panels.map((panel) => {
            const isOpen = open === panel.key;
            return (
              <button
                type="button"
                key={panel.key}
                data-testid="fr-panel"
                className={`cdt-fr-panel${panel.signalEdge ? ' cdt-fr-panel--sig' : ''}`}
                style={{ animationDelay: `${String(panel.delaySec)}s` }}
                aria-expanded={isOpen}
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

                <span className="cdt-fr-panel-caption">{panel.caption}</span>

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
          {/* Ornament: it repeats what the raised SHOW WORKING label now says on all six. */}
          <span className="cdt-fr-footer">{EVIDENCE_FOOTER}</span>
        </div>
      </div>
    </div>
  );
}

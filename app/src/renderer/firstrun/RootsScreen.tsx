import { useState } from 'react';
import type { ReactElement } from 'react';
import {
  ADD_A_FOLDER_LABEL,
  CONFIRM_LARGE_BODY,
  CONFIRM_LARGE_LABEL,
  CONSENT_HEADING,
  CONSENT_PARAGRAPH,
  CONSENT_ROWS,
  DIG_INERT_NOTE,
  DIG_LABEL,
  DIG_NOTE,
  EXCLUSION_HEADING,
  ROOTS_EYEBROW,
  ROOTS_HEADLINE,
  ROOTS_LIST_CAPTION,
  ROOTS_STANDFIRST,
  WHAT_EXACTLY_LABEL,
} from './copy';
import {
  EXCLUSION_CAPTION,
  EXCLUSION_CHIPS_BEFORE_EXPANDER,
  EXCLUSION_LIST,
} from '../../shared/skipList';
import type { RootRow } from './rootRows';
import type { ResolvedTier } from '../motion/tier';

/** §10.1b's fourth refusal, the confirmable one. */
export interface PendingConfirm {
  readonly pathDisplay: string;
  readonly estimatedDirs: number;
}

export interface RootsScreenDeps {
  readonly onToggleRoot: (key: string) => void;
  readonly onConsent: (granted: boolean) => void;
  readonly onAddFolder: () => void;
  readonly onConfirmLarge: (pathDisplay: string) => void;
  readonly onDig: () => void;
}

export interface RootsScreenProps {
  readonly deps: RootsScreenDeps;
  /** Already formatted by `toRow`/`refusedRow`; this screen derives nothing from a count. */
  readonly rows: readonly RootRow[];
  readonly ticked: ReadonlySet<string>;
  readonly consented: boolean;
  /** The one refusal that is a question rather than a verdict. `null` when none is open. */
  readonly pendingConfirm: PendingConfirm | null;
  readonly tier: ResolvedTier;
  /** True while the ticks are committing, so one press cannot start two scans. */
  readonly busy: boolean;
}

/** §10.1b: three rows of chips fit; the remainder expand in place behind one chip. */
export function visibleExclusions(expanded: boolean): readonly string[] {
  return expanded ? EXCLUSION_LIST : EXCLUSION_LIST.slice(0, EXCLUSION_CHIPS_BEFORE_EXPANDER);
}

export function moreChipLabel(hidden: number): string {
  return `+ ${String(hidden)} MORE`;
}

export function RootsScreen(props: RootsScreenProps): ReactElement {
  const { deps, rows, ticked, consented, pendingConfirm, tier, busy } = props;
  const [expanded, setExpanded] = useState(false);
  const [whatExactly, setWhatExactly] = useState(false);
  const hidden = EXCLUSION_LIST.length - EXCLUSION_CHIPS_BEFORE_EXPANDER;
  const inert = !consented || busy;

  return (
    <div className="cdt-fr-view cdt-fr-view--roots" data-effects-tier={tier}>
      <div className="cdt-fr-column">
        <span className="cdt-fr-eyebrow">{ROOTS_EYEBROW}</span>
        <h1 className="cdt-fr-headline">{ROOTS_HEADLINE}</h1>
        <p className="cdt-fr-standfirst">{ROOTS_STANDFIRST}</p>

        <div className="cdt-fr-roots" role="group" aria-label={ROOTS_LIST_CAPTION}>
          {rows.map((row) => (
            <RootRowView
              key={row.key}
              row={row}
              on={ticked.has(row.key)}
              onToggle={() => {
                deps.onToggleRoot(row.key);
              }}
            />
          ))}
        </div>
        <p className="cdt-fr-note">{ROOTS_LIST_CAPTION}</p>
        <button type="button" className="cdt-fr-chip cdt-fr-chip--more" onClick={deps.onAddFolder}>
          {ADD_A_FOLDER_LABEL}
        </button>

        {/* §10.1b's confirmable refusal. Nothing but the display string the core produced
            crosses back; the estimate is the core's, taken on a dialog pick and never on a
            suggested row. */}
        {pendingConfirm !== null ? (
          <div className="cdt-fr-confirm" role="group" aria-label={CONFIRM_LARGE_LABEL}>
            <p className="cdt-fr-standfirst">{CONFIRM_LARGE_BODY(pendingConfirm.estimatedDirs)}</p>
            <button
              type="button"
              className="cdt-fr-primary"
              onClick={() => {
                deps.onConfirmLarge(pendingConfirm.pathDisplay);
              }}
            >
              {CONFIRM_LARGE_LABEL}
            </button>
          </div>
        ) : null}

        <div className="cdt-fr-panels-two">
          <section>
            <div className="cdt-fr-provenance">{EXCLUSION_HEADING}</div>
            <div className="cdt-fr-chips">
              {visibleExclusions(expanded).map((entry) => (
                <span className="cdt-fr-chip" key={entry}>
                  {entry}
                </span>
              ))}
              {!expanded && hidden > 0 ? (
                <button
                  type="button"
                  className="cdt-fr-chip cdt-fr-chip--more"
                  onClick={() => {
                    setExpanded(true);
                  }}
                >
                  {moreChipLabel(hidden)}
                </button>
              ) : null}
            </div>
            <p className="cdt-fr-standfirst">{EXCLUSION_CAPTION}</p>
          </section>

          <section data-testid="fr-consent">
            <div className="cdt-fr-provenance">{CONSENT_HEADING}</div>
            {CONSENT_ROWS.map((row) =>
              row.kind === 'control' ? (
                <button
                  type="button"
                  key={row.note}
                  className="cdt-fr-consent-row"
                  data-kind="control"
                  data-on={consented}
                  role="checkbox"
                  aria-checked={consented}
                  onClick={() => {
                    deps.onConsent(!consented);
                  }}
                >
                  <span className="cdt-fr-tick" data-on={consented} />
                  <span>
                    <span className="cdt-fr-consent-body">{row.body}</span>
                    <span className="cdt-fr-consent-note">{row.note}</span>
                  </span>
                </button>
              ) : (
                <div key={row.note} className="cdt-fr-consent-row" data-kind="statement">
                  <span className="cdt-fr-consent-dot" data-testid="fr-consent-dot" />
                  <span>
                    <span className="cdt-fr-consent-body">{row.body}</span>
                    <span className="cdt-fr-consent-note">{row.note}</span>
                  </span>
                </div>
              ),
            )}
            <button
              type="button"
              className="cdt-fr-chip cdt-fr-chip--more"
              aria-expanded={whatExactly}
              onClick={() => {
                setWhatExactly((open) => !open);
              }}
            >
              {WHAT_EXACTLY_LABEL}
            </button>
            {whatExactly ? <p className="cdt-fr-standfirst">{CONSENT_PARAGRAPH}</p> : null}
          </section>
        </div>

        <div className="cdt-fr-commit">
          <button
            type="button"
            className="cdt-fr-primary"
            aria-disabled={inert}
            onClick={() => {
              if (!inert) deps.onDig();
            }}
          >
            {DIG_LABEL}
          </button>
          <span className="cdt-fr-note">{consented ? DIG_NOTE : DIG_INERT_NOTE}</span>
        </div>
      </div>
    </div>
  );
}

function RootRowView(props: {
  readonly row: RootRow;
  readonly on: boolean;
  readonly onToggle: () => void;
}): ReactElement {
  const { row, on, onToggle } = props;
  const body = (
    <>
      {/* §10.1b: the three absolute refusals are listed with no tick slot at all. */}
      {row.tickable ? <span className="cdt-fr-tick" data-on={on} /> : null}
      <span className="cdt-fr-row-body">
        <span className="cdt-fr-path" data-on={row.tickable && on}>
          {row.pathDisplay}
        </span>
        <span className="cdt-fr-provenance">{row.provenance}</span>
      </span>
      <span className="cdt-fr-count">
        {row.count}
        <span className="cdt-fr-count-unit">{row.countUnit}</span>
      </span>
    </>
  );
  return row.tickable ? (
    <button
      type="button"
      className="cdt-fr-root-row"
      data-testid="fr-root-row"
      data-tickable="true"
      role="checkbox"
      aria-checked={on}
      onClick={onToggle}
    >
      {body}
    </button>
  ) : (
    <div className="cdt-fr-root-row" data-testid="fr-root-row" data-tickable="false">
      {body}
    </div>
  );
}

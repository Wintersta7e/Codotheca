import { useEffect, useRef } from 'react';
import type { ReactElement } from 'react';
import { NOT_NOW_LABEL, SHOW_ME_LABEL } from './copy';
import { turnModel } from './turn';
import type { TurnCounts } from './turn';
import type { ResolvedTier } from '../motion/tier';

export interface TurnScreenProps {
  readonly counts: TurnCounts;
  /** Newest worktree observation across the counted projects; null when none was recorded. */
  readonly worktreeObservedAt: number | null;
  readonly tier: ResolvedTier;
  /**
   * §10.4a: the host sets `query_text` to this, **expands every era section and resets
   * density**. The expand and the reset are shelf state and are not this component's to write.
   * Rung 4's query is the empty string, which is the unfiltered shelf and not a no-op.
   */
  readonly onShowMe: (query: string) => void;
  readonly onNotNow: () => void;
}

/** §10.4a's turn. One line, one button, one way out — and nothing else on the screen. */
export function TurnScreen(props: TurnScreenProps): ReactElement {
  const model = turnModel(props.counts, props.worktreeObservedAt);
  const showMe = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    showMe.current?.focus();
  }, []);

  return (
    <div className="cdt-fr-view cdt-fr-view--turn" data-effects-tier={props.tier}>
      <div className="cdt-fr-turn-lede">
        <p className="cdt-fr-turn-line">{model.line}</p>
        {/* Rung 4 owes no qualifier, and an empty element would still take its place in the
            26px rhythm. */}
        {model.qualifier === null ? null : (
          <p className="cdt-fr-turn-qualifier">{model.qualifier}</p>
        )}
      </div>
      <button
        ref={showMe}
        type="button"
        className="cdt-fr-turn-show-me"
        onClick={() => {
          props.onShowMe(model.query);
        }}
      >
        {SHOW_ME_LABEL}
      </button>
      <button
        type="button"
        className="cdt-fr-turn-not-now"
        onClick={() => {
          props.onNotNow();
        }}
      >
        {NOT_NOW_LABEL}
      </button>
    </div>
  );
}

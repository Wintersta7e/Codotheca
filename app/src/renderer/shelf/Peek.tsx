import type { ReactElement } from 'react';
import type { Peek } from '../../generated/protocol.js';
import { allowsTransforms, type ResolvedTier } from '../motion/tier.js';
import {
  commitDate,
  firstParagraph,
  peekFacts,
  readmeFallback,
  shortSha,
  worktreeLine,
} from './peekText.js';

export const PEEK_ENTER_CLASS = 'cdt-peek--enter';

export interface PeekPanelProps {
  /** The generated wire type. R14: the generated name wins, so the component is `PeekPanel`. */
  readonly peek: Peek | null;
  readonly now: number;
  readonly tier: ResolvedTier;
}

export function PeekPanel({ peek, now, tier }: PeekPanelProps): ReactElement {
  const className = `cdt-peek${allowsTransforms(tier) ? ` ${PEEK_ENTER_CLASS}` : ''}`;
  const header = (
    <div className="cdt-peek-head">
      <span>PEEK</span>
      <span className="cdt-peek-rule" aria-hidden="true" />
      <span className="cdt-peek-path">{peek?.location?.pathDisplay ?? ''}</span>
    </div>
  );

  if (peek === null) {
    return (
      <section className={className} aria-label="Peek">
        {header}
      </section>
    );
  }

  const observation = worktreeLine(peek.worktree, now);

  return (
    <section className={className} aria-label="Peek">
      {header}
      <p className="cdt-peek-readme">
        {readmeFallback(peek.readme) ?? firstParagraph(peek.readme.text ?? '')}
      </p>
      {peek.interruptedOp === null ? null : (
        <span className="cdt-chip" data-chip="interrupted">
          INTERRUPTED
        </span>
      )}
      <div className="cdt-peek-commits">
        {peek.commits.slice(0, 3).map((commit) => (
          <div className="cdt-peek-commit" key={commit.sha}>
            <span className="cdt-peek-sha">{shortSha(commit.sha)}</span>
            <span className="cdt-peek-subject">{commit.subject}</span>
            <span className="cdt-peek-date">{commitDate(commit)}</span>
          </div>
        ))}
      </div>
      {observation === null ? null : <p className="cdt-peek-observation">{observation}</p>}
      <dl className="cdt-peek-facts">
        {peekFacts(peek, now).map((fact) => (
          <div key={fact.key}>
            <dt className="cdt-peek-fact-key">{fact.key}</dt>
            <dd className="cdt-peek-fact-value">{fact.value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}

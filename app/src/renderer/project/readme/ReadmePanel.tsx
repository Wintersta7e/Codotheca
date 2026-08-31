/**
 * §8.5.3. §8.4's ruling is not relaxed by giving the README more room: the page adds width, not
 * a parser. The text is a React text child, which escapes it — criterion 42.
 *
 * §8.5.3 also puts the read's observation age in the header's right-hand slot, and the wire
 * carries it: `ReadmeState.readAt` is `peek_cache.computed_at`, set by the core's one README
 * producer. Where nothing has read there is no age, so the slot is not drawn at all — an age is
 * a fact about a read that happened, and inventing one is the currency invariant inverted.
 */
import type { CSSProperties, ReactElement } from 'react';

import type { ProjectRow, ReadmeState } from '../../../generated/protocol';
import { appearanceFor, fadeFor, seedOf } from '../../art/appearance';
import { formatAge } from '../../derive/observation';
// §8.4's second string, and Peek renders the same sentence: one owner, re-exported here so this
// panel's callers can name it without a second copy of the bytes (R12).
import { README_ABSENT } from '../../shelf/peekText';
import { cascadeDelay } from '../motion';

export { README_ABSENT };

/** A promise: the pass has not run. §8.5.3, with v1's dead "tier" vocabulary removed. */
export const README_NOT_INDEXED = 'No README paragraph indexed yet — waiting on the content pass.';

export interface ReadmePanelProps {
  readme: ReadmeState;
  row: ProjectRow;
  /** Unix seconds, from `deps.now()`. §8.5.3's age slot is the only reader. */
  now: number;
}

export function ReadmePanel({ readme, row, now }: ReadmePanelProps): ReactElement {
  const text =
    readme.state === 'present' && readme.text !== null && readme.text !== ''
      ? readme.text
      : readme.state === 'absent'
        ? README_ABSENT
        : README_NOT_INDEXED;

  const appearance = appearanceFor(seedOf(row), fadeFor(row), row.primaryLanguage);

  return (
    <section
      className="cp-readme cp-rise"
      style={{ animationDelay: cascadeDelay(2), '--cdt-jewel': appearance.jewel } as CSSProperties}
    >
      <div className="cp-readme-header" data-testid="cp-readme-header">
        <span data-testid="cp-readme-name-slot">README.md</span>
        {readme.readAt === null ? null : (
          <span className="cp-readme-age" data-testid="cp-readme-age">
            {formatAge(now - readme.readAt)}
          </span>
        )}
      </div>
      <div className="cp-readme-body">
        <div className="cp-readme-name">{row.name}</div>
        <div className="cp-readme-stripe" aria-hidden="true" />
        <p className="cp-readme-text" data-testid="cp-readme-body">
          {text}
        </p>
      </div>
    </section>
  );
}

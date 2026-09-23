/**
 * [p3] §31.7's ten-row checklist, mounted in the `HEALTH` tab body.
 *
 * **§30.7 alone decides whether the tab mounts** (R130/F7). This component renders nothing when
 * `completion` is NULL and takes no view on the tab's presence — §31.8 reads as *the checklist
 * renders its ten rows or the tab carries none*, and the second half is §30's sentence.
 *
 * **Decision-carrying text sits at `--text-3` or lighter.**
 */
import type { ReactElement } from 'react';

import type { CompletionDetail } from '../../../generated/protocol';
import { PartSeparator } from '../health/PartSeparator';
import { CHECK_LABELS, markFor, noteFor } from './checklist';

/** The design's own name for this block, as it heads it in the `HEALTH` tab. */
export const COMPLETION_TITLE = 'COMPLETION';

export interface CompletionChecklistProps {
  readonly completion: CompletionDetail | null;
}

export function CompletionChecklist({ completion }: CompletionChecklistProps): ReactElement | null {
  if (completion === null) return null;

  return (
    <section
      className="cp-completion"
      data-testid="cp-completion"
      aria-label="Completion checklist"
    >
      <h3 className="cp-completion-title">{COMPLETION_TITLE}</h3>
      <ul className="cp-completion-checks">
        {completion.checks.map((row) => {
          const mark = markFor(row);
          const note = noteFor(row);
          return (
            <li
              key={row.key}
              className="cp-completion-check"
              data-check={row.key}
              data-state={row.state}
            >
              {/* Separated in the text, as the rest of the tab is: `□ REMOTE`, never `□REMOTE`,
                  and `PUSHED · UNKNOWN · NOT OBSERVED`, never `PUSHEDUNKNOWN`. */}
              <span className="cp-completion-mark" data-hollow={mark.hollow} aria-hidden="true">
                {mark.glyph}
              </span>{' '}
              <span className="cp-completion-label">{CHECK_LABELS[row.key]}</span>
              {note === null ? null : (
                <>
                  <PartSeparator />
                  <span className="cp-completion-note">{note}</span>
                </>
              )}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

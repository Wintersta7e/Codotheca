/**
 * §8.5.3, and [p2] §25.5: the panel renders the document rather than the paragraph.
 *
 * §8.4's ruling still binds **Peek**, which stays plain text — mounting a document per expanded
 * card on the surface measured at 1,000 virtualized cards is a different decision with different
 * evidence. What moves here is the project page's half: the `<p>` becomes a sandboxed frame, and
 * the markup is parsed with `html: false`, sanitised against an explicit allowlist and serialised
 * into a document with an opaque origin and every sandbox flag off.
 *
 * **Nothing in this file imports the pipeline statically.** `useReadmeDocument` reaches it through
 * one dynamic `import()`, which is what keeps `markdown-it`, DOMPurify, `highlight.js` and KaTeX
 * off the first-paint chunk; even the frame's height comes from a module of its own for that
 * reason.
 *
 * §8.5.3 also puts the read's observation age in the header's right-hand slot, and the wire
 * carries it: `ReadmeState.readAt` is `peek_cache.computed_at`, set by the core's one README
 * producer. Where nothing has read there is no age, so the slot is not drawn at all — an age is
 * a fact about a read that happened, and inventing one is the currency invariant inverted.
 */
import type { CSSProperties, ReactElement } from 'react';

import type { LocationId, ProjectRow, ReadmeState } from '../../../generated/protocol';
import { appearanceFor, fadeFor, seedOf } from '../../art/appearance';
import { formatAge } from '../../derive/observation';
// §8.4's second string, and Peek renders the same sentence: one owner, re-exported here so this
// panel's callers can name it without a second copy of the bytes (R12).
import { README_ABSENT } from '../../shelf/peekText';
import { cascadeDelay } from '../motion';
import { FRAME_HEIGHT_PX } from './frameHeight';
import { useReadmeDocument } from './useReadmeDocument';

export { README_ABSENT };

/** A promise: the pass has not run. §8.5.3, with v1's dead "tier" vocabulary removed. */
export const README_NOT_INDEXED = 'No README paragraph indexed yet — waiting on the content pass.';

/** [p2] §25.5's statement, below the frame, where the document has at least one anchor. */
export const LINKS_INERT_STATEMENT = 'LINKS ARE NOT ACTIVE IN THIS PANEL';

/** [p2] §25.5: a document that was cut says so rather than appearing to end. */
export const README_TRUNCATED_STATEMENT = 'READ TO THE SIZE CAP — THE DOCUMENT CONTINUES';

/**
 * [p2] §25.5's consent sentence and its one control. **`blocked` is the consent state and is
 * never rendered as a failure**: no error ink, no warning plate.
 */
export const REMOTE_BLOCKED_STATEMENT = 'REMOTE IMAGES ARE NOT LOADED FOR THIS REPOSITORY';
export const REMOTE_GRANT_LABEL = 'LOAD THEM';

/** The name slot's fallback, for a read that did not say which file it found. */
const README_DEFAULT_NAME = 'README.md';

export interface ReadmePanelProps {
  readme: ReadmeState;
  row: ProjectRow;
  /** Unix seconds, from `deps.now()`. §8.5.3's age slot is the only reader. */
  now: number;
  /**
   * [p2] §25.5: which copy the document is read from. `null` — a project with no location — draws
   * §8.5.3's strings and asks for nothing, which is §23's case and not this panel's to rule on.
   */
  locationId?: LocationId | null;
  /**
   * [p2] §25.3's topic chips, restored with the chrome §8.5.3 already specifies. **Zero topics
   * renders no row at all**, never an empty rail — §5.6's *nothing selected, no block renders*,
   * and an empty rail is furniture.
   *
   * It is `readonly string[]` and not `RemoteFacts` deliberately: §25.4's gate keeps the forge
   * types out of every surface that would then acquire a reason to score them, and this panel
   * already has one source gate reading it.
   */
  topics?: readonly string[];
}

export function ReadmePanel({
  readme,
  row,
  now,
  locationId = null,
  topics = [],
}: ReadmePanelProps): ReactElement {
  const document = useReadmeDocument({ projectId: row.id, locationId, readme });

  const text =
    readme.state === 'present' && readme.text !== null && readme.text !== ''
      ? readme.text
      : readme.state === 'absent'
        ? README_ABSENT
        : README_NOT_INDEXED;

  const appearance = appearanceFor(seedOf(row), fadeFor(row), row.primaryLanguage);
  const srcdoc = document.srcdoc;
  const framed = document.phase === 'frame' && srcdoc !== null;

  return (
    <section
      className="cp-readme cp-rise"
      style={{ animationDelay: cascadeDelay(2), '--cdt-jewel': appearance.jewel } as CSSProperties}
    >
      <div className="cp-readme-header" data-testid="cp-readme-header">
        <span data-testid="cp-readme-name-slot">{document.path ?? README_DEFAULT_NAME}</span>
        {readme.readAt === null ? null : (
          <span className="cp-readme-age" data-testid="cp-readme-age">
            {formatAge(now - readme.readAt)}
          </span>
        )}
      </div>
      <div className="cp-readme-body">
        <div className="cp-readme-name">{row.name}</div>
        <div className="cp-readme-stripe" aria-hidden="true" />
        {framed && srcdoc !== null ? (
          <iframe
            className="cp-readme-frame"
            data-testid="cp-readme-frame"
            title={`${row.name} README`}
            sandbox=""
            srcDoc={srcdoc}
            style={{ height: `${String(FRAME_HEIGHT_PX)}px` }}
          />
        ) : (
          <p className="cp-readme-text" data-testid="cp-readme-body">
            {text}
          </p>
        )}
        {framed && document.truncated ? (
          <p className="cp-readme-statement" data-testid="cp-readme-truncated">
            {README_TRUNCATED_STATEMENT}
          </p>
        ) : null}
        {framed && document.anchorCount > 0 ? (
          <p className="cp-readme-statement" data-testid="cp-readme-links-inert">
            {LINKS_INERT_STATEMENT}
          </p>
        ) : null}
        {framed && document.blockedRemote > 0 ? (
          <div className="cp-readme-consent" data-testid="cp-readme-consent">
            <span>{REMOTE_BLOCKED_STATEMENT}</span>
            <button type="button" onClick={document.grantRemote}>
              {REMOTE_GRANT_LABEL}
            </button>
          </div>
        ) : null}
        {topics.length === 0 ? null : (
          <div className="cp-readme-topics" data-testid="cp-readme-topics">
            {topics.map((topic) => (
              <span className="cp-readme-topic" key={topic}>
                {topic}
              </span>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}

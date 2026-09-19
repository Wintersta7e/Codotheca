import type { ReactElement } from 'react';
import type { Peek, RemoteFacts } from '../../generated/protocol.js';
import { allowsTransforms, type ResolvedTier } from '../motion/tier.js';
import { observationLine, RemoteBlock } from '../project/remote/remoteBlocks.js';
import {
  commitDate,
  firstParagraph,
  peekFacts,
  readmeFallback,
  shortSha,
  worktreeLine,
  type PeekCompletion,
} from './peekText.js';

export const PEEK_ENTER_CLASS = 'cdt-peek--enter';

export interface PeekPanelProps {
  /** The generated wire type. R14: the generated name wins, so the component is `PeekPanel`. */
  readonly peek: Peek | null;
  readonly now: number;
  readonly tier: ResolvedTier;
  /**
   * [p3] §31.7's sixth fact, from the shelf row this Peek was opened beside. `null` renders the
   * uncomputed mark, which is the honest reading of a row the caller could not name.
   */
  readonly completion?: PeekCompletion | null;
}

export function PeekPanel({ peek, now, tier, completion }: PeekPanelProps): ReactElement {
  const className = `cdt-peek${allowsTransforms(tier) ? ` ${PEEK_ENTER_CLASS}` : ''}`;
  // [p2] §25.3a: **absent**, and not filled with the remote key. A path slot holding a URL
  // invites the one gesture the row cannot serve.
  const header = (
    <div className="cdt-peek-head">
      <span>PEEK</span>
      <span className="cdt-peek-rule" aria-hidden="true" />
      {peek === null || peek.location === null ? null : (
        <span className="cdt-peek-path">{peek.location.pathDisplay}</span>
      )}
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
  // [p2] §25.3a: a row with no working copy renders **a different set**, not the same five with
  // dashes in them. The README paragraph, the commit list, the path and the worktree line are
  // absent — each of them is a claim about a repository nothing has read or a disk nothing has
  // looked at.
  const cloned = peek.location !== null;

  return (
    <section className={className} aria-label="Peek">
      {header}
      {cloned ? (
        <p className="cdt-peek-readme">
          {readmeFallback(peek.readme) ?? firstParagraph(peek.readme.text ?? '')}
        </p>
      ) : null}
      {peek.interruptedOp === null ? null : (
        <span className="cdt-chip" data-chip="interrupted">
          INTERRUPTED
        </span>
      )}
      {cloned ? (
        <div className="cdt-peek-commits">
          {peek.commits.slice(0, 3).map((commit) => (
            <div className="cdt-peek-commit" key={commit.sha}>
              <span className="cdt-peek-sha">{shortSha(commit.sha)}</span>
              <span className="cdt-peek-subject">{commit.subject}</span>
              <span className="cdt-peek-date">{commitDate(commit)}</span>
            </div>
          ))}
        </div>
      ) : null}
      {cloned && observation !== null ? (
        <p className="cdt-peek-observation">{observation}</p>
      ) : null}
      <dl className="cdt-peek-facts">
        {peekFacts(peek, now, completion ?? null).map((fact) => (
          <div key={fact.key}>
            <dt className="cdt-peek-fact-key">{fact.key}</dt>
            <dd className="cdt-peek-fact-value">{fact.value}</dd>
          </div>
        ))}
      </dl>
      {/*
        §25.3a's authority is the **not-cloned** row and only that: the blocks stand *in place of*
        the five facts a row with no copy has no history for. A cloned row already has those five,
        and adding the key, the visibility word and three blocks to it widens §8.4.1 — a phase-1
        surface this plan does not own, and one that deliberately refuses roast, notes and
        completion. Gated on the same predicate §25.3a is written against rather than on
        `remote !== null`, which is a different question.
      */}
      {peek.location === null ? <PeekRemote remote={peek.remote} now={now} /> : null}
    </section>
  );
}

/**
 * §25.3a's **positive half**: what a not-cloned row renders in place of the facts it has no
 * history for — the key, visibility, and stars / open issues / open PRs where observed, under
 * §25.1's four states and its observation line.
 *
 * `RemoteBlock` is **imported, not re-implemented**: this is the same producer at Peek's size,
 * which is what §25.3a asks for and what §25.4's import gate admits this module for. A cloned
 * project's Peek gains the same blocks and that is not a regression — the five facts are
 * unchanged for a row with a location.
 *
 * §24 owns the install control; Peek adds none of its own.
 */
function PeekRemote({
  remote,
  now,
}: {
  readonly remote: RemoteFacts | null;
  readonly now: number;
}): ReactElement | null {
  if (remote === null) return null;
  const observed = observationLine(remote.observedAt, now);
  const visibility =
    remote.visibility === 'public' ? 'PUBLIC' : remote.visibility === 'private' ? 'PRIVATE' : null;

  return (
    <div className="cdt-peek-remote" data-testid="cdt-peek-remote">
      <span className="cdt-peek-remote-key" data-testid="cdt-peek-remote-key">
        {remote.key}
      </span>
      {visibility === null ? null : (
        <span className="cdt-peek-remote-visibility">{visibility}</span>
      )}
      <div className="cdt-peek-remote-blocks">
        <RemoteBlock label="STARS" state={remote.state} value={remote.stars} />
        <RemoteBlock label="OPEN ISSUES" state={remote.state} value={remote.openIssues} />
        <RemoteBlock label="OPEN PRS" state={remote.state} value={remote.openPrs} />
      </div>
      {observed === null ? null : (
        <p className="cdt-peek-remote-observed" data-testid="cdt-peek-remote-observed">
          {observed}
        </p>
      )}
    </div>
  );
}

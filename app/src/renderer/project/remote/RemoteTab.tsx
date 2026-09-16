/**
 * §25.1's `REMOTE` tab, in the design's order: header · fork line · `OPEN ISSUES` · `OPEN PRS` ·
 * `STARS` · `BEHIND` · the observation line · `LATEST CI` · the links row.
 *
 * **`BEHIND` is the local figure and needs no account**, which is what gives this tab content
 * with nothing connected and keeps it from being the surface that exists to advertise a feature.
 *
 * §25 declares no new duration and no new easing curve: the tab enters on §8.5.1's existing
 * cascade, so `css-vocabulary.json` is untouched.
 */
import type { ReactElement } from 'react';

import type {
  LocationDetail,
  ProjectId,
  RemoteFacts,
  RemoteLinkKind,
} from '../../../generated/protocol';
import { behindFact } from './behindBlock';
import { ciInk, ciLabel, CI_RUN_LIMIT } from './ciCopy';
import { RemoteLinks } from './links';
import { NOT_PERMITTED, NOT_YET_FETCHED, observationLine, RemoteBlock } from './remoteBlocks';

/**
 * §25.1's one statement for a surface with nothing connected.
 *
 * It is a **statement and not a control**: this page carries no navigation seam, so a button
 * here could not reach the accounts surface and §11.3a forbids one that cannot act. Naming where
 * the setting lives is the honest half of the affordance the section asks for.
 */
export const NO_ACCOUNT_STATEMENT =
  'No account is connected. Issues, pull requests, stars and CI runs are read from the forge, and nothing here has been read yet.';

export interface RemoteTabProps {
  readonly projectId: ProjectId;
  readonly facts: RemoteFacts;
  /** The location the page is showing — §25.1's `BEHIND` is that copy's figure. */
  readonly shown: LocationDetail | null;
  readonly now: number;
  readonly onOpenLink: (projectId: ProjectId, kind: RemoteLinkKind) => void;
}

export function RemoteTab({
  projectId,
  facts,
  shown,
  now,
  onOpenLink,
}: RemoteTabProps): ReactElement {
  const connected = facts.state !== 'no_account';
  const behind = behindFact(shown, now);
  const observed = observationLine(facts.observedAt, now);
  const ciObserved = observationLine(facts.ci.observedAt, now);

  // Each sub-line has its own three-way answer: a number, an absence, or **no sub-line at all**
  // because the value was never observed. A sub-line printed from a NULL is the invariant's
  // exact failure mode, so every one of these is `null` rather than a zero-derived string.
  const goodFirst =
    facts.goodFirstIssues === null || facts.goodFirstIssues === 0
      ? null
      : `${String(facts.goodFirstIssues)} labelled good-first-issue`;
  // `OPEN PRS` differs from `OPEN ISSUES` deliberately: its zero has a sentence and is rendered,
  // where the issues sub-line is absent at zero. Asserting one shape for both loses that.
  const fromYou =
    facts.openPrsFromUser === null
      ? null
      : facts.openPrsFromUser === 0
        ? 'none from you'
        : `${String(facts.openPrsFromUser)} from you`;

  return (
    <section className="cp-remote" data-testid="cp-remote">
      <header className="cp-remote-head">
        <span className="cp-remote-title">REMOTE</span>
        <span className="cp-remote-key" data-testid="cp-remote-key">
          {facts.key}
        </span>
      </header>

      {/* §25.2: text, never a link. The opener reconstructs from **this** project's key, and a
          second derivation source is a widening phase 2 does not make. */}
      {facts.forkParentKey === null ? null : (
        <p className="cp-remote-fork" data-testid="cp-remote-fork">
          {`FORK OF ${facts.forkParentKey}`}
        </p>
      )}

      {connected ? null : (
        <p className="cp-remote-statement" data-testid="cp-remote-no-account">
          {NO_ACCOUNT_STATEMENT}
        </p>
      )}

      <div className="cp-remote-blocks">
        <RemoteBlock
          label="OPEN ISSUES"
          state={facts.state}
          value={facts.openIssues}
          subLine={goodFirst}
        />
        <RemoteBlock label="OPEN PRS" state={facts.state} value={facts.openPrs} subLine={fromYou} />
        {/* No `+2 this month`: a delta cannot be computed from a first observation, and `+0` on
            connect day is an unknown rendered as zero. */}
        <RemoteBlock label="STARS" state={facts.state} value={facts.stars} subLine={null} />
        {behind === null ? null : (
          <RemoteBlock
            label="BEHIND"
            state="observed"
            value={behind.behind}
            subLine={behind.subLine}
          />
        )}
      </div>

      {connected && observed !== null ? (
        <p className="cp-remote-observed" data-testid="cp-remote-observed">
          {observed}
        </p>
      ) : null}

      {connected ? (
        <div className="cp-remote-ci" data-testid="cp-remote-ci">
          <span className="cp-remote-block-label">LATEST CI</span>
          {facts.ci.state === 'observed' ? (
            <ul className="cp-remote-ci-list">
              {facts.ci.runs.slice(0, CI_RUN_LIMIT).map((run) => (
                <li className="cp-remote-ci-row" key={run.runId}>
                  <span className="cp-remote-ci-left">
                    <span className="cp-remote-ci-workflow">{run.workflow}</span>
                    <span className="cp-remote-ci-dash" aria-hidden="true">
                      —
                    </span>
                    <span
                      className="cp-remote-ci-conclusion"
                      data-ink={ciInk(run.conclusion)}
                      style={{ color: `var(${ciInk(run.conclusion)})` }}
                    >
                      {ciLabel(run.conclusion)}
                    </span>
                  </span>
                  {/* The branch is printed **once**, on the right. The screenshot prints it
                      twice and §25.4 rules once. */}
                  <span className="cp-remote-ci-right">
                    {`run #${String(run.runNumber)} · ${run.branch}`}
                  </span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="cp-remote-ci-state" data-testid="cp-remote-ci-state">
              {facts.ci.state === 'not_permitted' ? NOT_PERMITTED : NOT_YET_FETCHED}
            </p>
          )}
          {ciObserved === null ? null : (
            <p className="cp-remote-ci-observed" data-testid="cp-remote-ci-observed">
              {ciObserved}
            </p>
          )}
        </div>
      ) : null}

      <RemoteLinks projectId={projectId} facts={facts} onOpenLink={onOpenLink} />
    </section>
  );
}

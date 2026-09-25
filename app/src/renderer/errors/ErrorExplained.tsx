/**
 * §11.1's per-project error state. "Never render unknown as zero" has a partner: render unknown
 * as **explained**, somewhere reachable. This is that place, on the tile and on the page.
 *
 * The component holds no transport of its own and composes no string: `request` and the two
 * callbacks arrive as props, so criterion 63's "`TRY AGAIN` is not wired to `scan.start`" is a
 * unit assertion, and every word comes from `errorKind.ts`.
 *
 * The type is written as `fontSize`/`letterSpacing` rather than the `font` shorthand because
 * `scripts/check-type-scale.mjs` reads the longhand only; the shorthand would put every size in
 * this file outside criterion 60's reach without anything saying so.
 */
import type { CSSProperties, ReactElement } from 'react';

import type { CommandArgs, CommandName, CommandResult, LocationId } from '../../generated/protocol';
import {
  ERROR_ACTION_LABEL,
  explainErrorKind,
  lastTriedLine,
  neverSucceeded,
  TRY_AGAIN_LABEL,
  type ErrorSubject,
} from './errorKind';

export type ErrorRequest = <K extends CommandName>(
  name: K,
  args: CommandArgs[K],
) => Promise<CommandResult[K]>;

export interface ErrorExplainedProps {
  readonly subject: ErrorSubject;
  readonly variant: 'tile' | 'page';
  readonly nowSecs: number;
  readonly request: ErrorRequest;
  readonly onRelocate: (locationId: LocationId) => void;
  readonly onChanged: () => void;
}

/** §11.1: the badge takes the design's `unk` treatment, not `--unknown`'s opaque pair. */
const badgeStyle: CSSProperties = {
  alignSelf: 'flex-start',
  padding: '2px 7px',
  background: 'rgb(30 38 46 / .95)',
  color: 'rgb(186 206 222 / .92)',
  fontFamily: 'var(--font-mono, ui-monospace)',
  fontSize: 8.5,
  fontWeight: 700,
  lineHeight: 1,
  letterSpacing: '.16em',
};

const proseStyle: CSSProperties = {
  margin: 0,
  maxWidth: '46ch',
  fontFamily: 'var(--font-body, system-ui)',
  fontSize: 12.5,
  lineHeight: 1.55,
  color: 'var(--text-2, #b6c1cb)',
  textWrap: 'pretty',
};

const noteStyle: CSSProperties = {
  fontFamily: 'var(--font-mono, ui-monospace)',
  fontSize: 8.5,
  lineHeight: 1,
  letterSpacing: '.14em',
  color: 'var(--text-3, #8b97a3)',
};

const outlineButton: CSSProperties = {
  height: 28,
  padding: '0 12px',
  background: 'transparent',
  border: '1px solid var(--line-5, #3c454e)',
  color: 'var(--text-2, #b6c1cb)',
  fontFamily: 'var(--font-display, system-ui)',
  fontSize: 12,
  fontWeight: 600,
  lineHeight: 1,
  letterSpacing: '.12em',
  cursor: 'pointer',
};

export function ErrorExplained(props: ErrorExplainedProps): ReactElement | null {
  const { subject, variant, nowSecs, request, onRelocate, onChanged } = props;
  const { errorKind } = subject;
  if (errorKind === null) return null;
  if (!neverSucceeded(subject)) return null;

  const explained = explainErrorKind(errorKind);
  if (explained.placement !== 'project' || explained.badge === null) return null;

  const tried = lastTriedLine(subject.errorAt, nowSecs);
  const onPage = variant === 'page';

  const tryAgain = (): void => {
    void request('projects.requeue', { id: subject.projectId }).then(onChanged, onChanged);
  };
  const trust = (locationId: LocationId): void => {
    void request('locations.setTrusted', { locationId }).then(onChanged, onChanged);
  };

  const action = explained.action;
  const locationId = subject.locationId;
  const showAction = onPage && action !== null && locationId !== null;

  return (
    <div
      data-testid="err-explained"
      style={{ display: 'flex', flexDirection: 'column', gap: onPage ? 9 : 6 }}
    >
      <span data-testid="err-badge" style={badgeStyle}>
        {explained.badge}
      </span>
      {onPage && explained.prose !== null ? (
        <p data-testid="err-prose" style={proseStyle}>
          {explained.prose}
        </p>
      ) : null}
      {tried !== null ? (
        <span data-testid="err-last-tried" style={noteStyle}>
          {tried}
        </span>
      ) : null}
      <div style={{ display: 'flex', gap: 8 }}>
        {showAction && action === 'trust' ? (
          <button
            type="button"
            style={outlineButton}
            onClick={() => {
              trust(locationId);
            }}
          >
            {ERROR_ACTION_LABEL.trust}
          </button>
        ) : null}
        {showAction && action === 'relocate' ? (
          <button
            type="button"
            style={outlineButton}
            onClick={() => {
              onRelocate(locationId);
            }}
          >
            {ERROR_ACTION_LABEL.relocate}
          </button>
        ) : null}
        <button type="button" style={outlineButton} onClick={tryAgain}>
          {TRY_AGAIN_LABEL}
        </button>
      </div>
    </div>
  );
}

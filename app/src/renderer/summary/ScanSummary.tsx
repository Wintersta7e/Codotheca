/**
 * §11.1's scan summary. Backed by `problems.list`, reachable from §10.2's scan line and from
 * §11.3a's drawer, and positioned by neither — the host sizes it.
 *
 * Two guards are deliberate duplicates of the core's: an empty group is not rendered, and a
 * zeroed header is not rendered. `problems.list` already enforces both; a regression on either
 * side would put `UNTRUSTED 0` on screen, which teaches the reader that a healthy machine is a
 * defective one.
 *
 * `problemCount` and `ambiguousLineageCount` are §11.1's arithmetic and are computed once, in
 * the core. Nothing here recounts them from `groups` — two answers to one question is how the
 * header and the list below it come to disagree in the same frame.
 */
import type { CSSProperties, ReactElement } from 'react';

import type { LocationId, ProblemGroup, ProjectId, Problems } from '../../generated/protocol';
import { ERROR_ACTION_LABEL } from '../errors/errorKind';
import type { ErrorRequest } from '../errors/ErrorExplained';
import {
  AMBIGUOUS_GROUP_NOTE,
  NO_PROBLEMS_HEADING,
  NO_SCAN_YET_BODY,
  NO_SCAN_YET_HEADING,
  OPEN_PROJECT_LABEL,
  PROBLEM_GROUP_LABEL,
  PROBLEMS_DISMISSED_HEADING,
  summaryDetailLine,
  summaryHeaderClauses,
} from './copy';

export interface ScanSummaryProps {
  readonly problems: Problems;
  /**
   * This run's banner was dismissed, so its problem list goes with it. Passed in rather than read
   * here: the dismissal lives in the view state, which is the shelf's, not this panel's.
   */
  readonly problemsDismissed?: boolean;
  readonly request: ErrorRequest;
  readonly onOpenProject: (id: ProjectId) => void;
  readonly onChanged: () => void;
}

const headerStyle: CSSProperties = {
  fontFamily: 'var(--font-mono, ui-monospace)',
  fontSize: 11,
  lineHeight: 1.4,
  letterSpacing: '.14em',
  color: 'var(--text-3, #8b97a3)',
};
/** §11.1: figures in `--text-1`, separators in `--text-4` and decorative only. */
const figureStyle: CSSProperties = { color: 'var(--text-1, #dde3e8)' };
const sepStyle: CSSProperties = { color: 'var(--text-4, #6c7885)' };

const groupLabelStyle: CSSProperties = {
  margin: 0,
  fontFamily: 'var(--font-mono, ui-monospace)',
  fontSize: 9,
  fontWeight: 700,
  lineHeight: 1,
  letterSpacing: '.22em',
  color: 'var(--text-2, #b6c1cb)',
};
const pathStyle: CSSProperties = {
  fontFamily: 'var(--font-mono, ui-monospace)',
  fontSize: 11.5,
  lineHeight: 1.4,
  letterSpacing: '.02em',
  color: 'var(--text-1, #dde3e8)',
  overflowWrap: 'anywhere',
};
const detailStyle: CSSProperties = {
  fontFamily: 'var(--font-body, system-ui)',
  fontSize: 11,
  lineHeight: 1.45,
  color: 'var(--text-3, #8b97a3)',
};
const noteStyle: CSSProperties = {
  margin: 0,
  borderLeft: '2px solid var(--line-5, #3c454e)',
  padding: '2px 0 2px 10px',
  maxWidth: '62ch',
  fontFamily: 'var(--font-body, system-ui)',
  fontSize: 11.5,
  lineHeight: 1.45,
  color: 'var(--text-2, #b6c1cb)',
  textWrap: 'pretty',
};
const outlineButton: CSSProperties = {
  alignSelf: 'flex-start',
  height: 26,
  padding: '0 11px',
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
const emptyHeading: CSSProperties = {
  margin: 0,
  fontFamily: 'var(--font-display, system-ui)',
  fontSize: 20,
  fontWeight: 700,
  lineHeight: 1.1,
  letterSpacing: '.06em',
  color: 'var(--text-2, #b6c1cb)',
};

function GroupRows(props: {
  readonly group: ProblemGroup;
  readonly request: ErrorRequest;
  readonly onOpenProject: (id: ProjectId) => void;
  readonly onChanged: () => void;
}): ReactElement {
  const { group, request, onOpenProject, onChanged } = props;
  const trust = (locationId: LocationId): void => {
    void request('locations.setTrusted', { locationId }).then(onChanged, onChanged);
  };
  return (
    <section
      data-testid={`sum-group-${group.kind}`}
      style={{ display: 'flex', flexDirection: 'column', gap: 9 }}
    >
      <h3 data-testid="sum-group-label" style={groupLabelStyle}>
        {`${PROBLEM_GROUP_LABEL[group.kind]} ${String(group.count)}`}
      </h3>
      {group.kind === 'ambiguous_lineage' ? (
        <p data-testid="sum-note" style={noteStyle}>
          {AMBIGUOUS_GROUP_NOTE}
        </p>
      ) : null}
      {group.items.map((item, index) => {
        const detail = summaryDetailLine(group.kind, item);
        const projectId = item.projectId;
        const locationId = item.locationId;
        return (
          <div
            key={`${item.pathDisplay}:${String(index)}`}
            style={{ display: 'flex', flexDirection: 'column', gap: 4 }}
          >
            <span style={pathStyle}>{item.pathDisplay}</span>
            {detail !== null ? (
              <span data-testid="sum-detail" style={detailStyle}>
                {detail}
              </span>
            ) : null}
            {group.kind === 'untrusted_repo' && locationId !== null ? (
              <button type="button" style={outlineButton} onClick={() => trust(locationId)}>
                {ERROR_ACTION_LABEL.trust}
              </button>
            ) : null}
            {group.kind === 'ambiguous_lineage' && projectId !== null ? (
              <button type="button" style={outlineButton} onClick={() => onOpenProject(projectId)}>
                {OPEN_PROJECT_LABEL}
              </button>
            ) : null}
          </div>
        );
      })}
    </section>
  );
}

export function ScanSummary(props: ScanSummaryProps): ReactElement {
  const { problems, problemsDismissed = false, request, onOpenProject, onChanged } = props;
  const clauses = summaryHeaderClauses(problems, problemsDismissed);
  const groups = problemsDismissed ? [] : problems.groups.filter((g) => g.items.length > 0);

  if (clauses === null) {
    return (
      <div style={{ display: 'flex', flexDirection: 'column', gap: 9 }}>
        <h2 style={emptyHeading}>{NO_SCAN_YET_HEADING}</h2>
        <p style={detailStyle}>{NO_SCAN_YET_BODY}</p>
      </div>
    );
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 24 }}>
      <p data-testid="sum-header" style={headerStyle}>
        {clauses.map((clause, index) => (
          <span key={clause.label}>
            {index > 0 ? <span style={sepStyle}>{' · '}</span> : null}
            <span style={figureStyle}>{clause.figure}</span>
            {` ${clause.label}`}
          </span>
        ))}
      </p>
      {groups.length === 0 ? (
        <h2 style={emptyHeading}>
          {problemsDismissed ? PROBLEMS_DISMISSED_HEADING : NO_PROBLEMS_HEADING}
        </h2>
      ) : (
        groups.map((group) => (
          <GroupRows
            key={group.kind}
            group={group}
            request={request}
            onOpenProject={onOpenProject}
            onChanged={onChanged}
          />
        ))
      )}
    </div>
  );
}

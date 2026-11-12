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
import {
  type CSSProperties,
  type KeyboardEvent,
  type ReactElement,
  useLayoutEffect,
  useRef,
  useState,
} from 'react';

import type { ProblemGroup, ProjectId, Problems, ScanProblemId } from '../../generated/protocol';
import { ERROR_ACTION_LABEL } from '../errors/errorKind';
import type { ErrorRequest } from '../errors/ErrorExplained';
import { CLOSE_LABEL } from '../firstrun/copy';
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
  /** `CLOSE` and `Esc`. Focus then returns to whatever opened the summary. */
  readonly onClose: () => void;
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

/**
 * The button the row is about to lose takes focus with it, so focus goes to a neighbouring
 * control in the summary first — the next one, else the one before — never to the document body.
 */
function focusNeighbour(button: HTMLButtonElement | null): void {
  const scope = button?.closest('section')?.parentElement;
  if (!button || !scope) return;
  const controls = Array.from(scope.querySelectorAll('button')).filter(
    (b) => b !== button && b.getAttribute('aria-disabled') !== 'true',
  );
  const after = controls.find(
    (b) => (button.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0,
  );
  (after ?? controls.at(-1))?.focus();
}

/**
 * §11.1's TRUST THIS REPOSITORY, keyed by the scan problem: an untrusted folder has no location
 * row to name. `problems.trust` is not idempotent, so a second press while the first is in
 * flight sends nothing.
 */
function TrustButton(props: {
  readonly problemId: ScanProblemId;
  readonly request: ErrorRequest;
  readonly onChanged: () => void;
}): ReactElement {
  const { problemId, request, onChanged } = props;
  const [pending, setPending] = useState(false);
  const self = useRef<HTMLButtonElement>(null);
  const trust = (): void => {
    if (pending) return;
    setPending(true);
    void request('problems.trust', { problemId }).then(
      () => {
        focusNeighbour(self.current);
        onChanged();
      },
      () => {
        setPending(false);
        onChanged();
      },
    );
  };
  return (
    <button ref={self} type="button" style={outlineButton} aria-disabled={pending} onClick={trust}>
      {ERROR_ACTION_LABEL.trust}
    </button>
  );
}

function GroupRows(props: {
  readonly group: ProblemGroup;
  readonly request: ErrorRequest;
  readonly onOpenProject: (id: ProjectId) => void;
  readonly onChanged: () => void;
}): ReactElement {
  const { group, request, onOpenProject, onChanged } = props;
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
        const problemId = item.problemId;
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
            {group.kind === 'untrusted_repo' && problemId !== null ? (
              <TrustButton problemId={problemId} request={request} onChanged={onChanged} />
            ) : null}
            {group.kind === 'ambiguous_lineage' && projectId !== null ? (
              <button
                type="button"
                style={outlineButton}
                onClick={() => {
                  onOpenProject(projectId);
                }}
              >
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
  const { problems, problemsDismissed = false, request, onOpenProject, onChanged, onClose } = props;
  const clauses = summaryHeaderClauses(problems, problemsDismissed);
  const groups = problemsDismissed ? [] : problems.groups.filter((g) => g.items.length > 0);
  const closeRef = useRef<HTMLButtonElement>(null);

  // Focus moves in on open, so `Esc` works at once, and goes back to the control that opened the
  // summary when it unmounts — whichever way it closed.
  useLayoutEffect(() => {
    const opener = document.activeElement;
    closeRef.current?.focus();
    return () => {
      if (opener instanceof HTMLElement && opener.isConnected) opener.focus();
    };
  }, []);

  // `Esc` closes the summary and nothing under it: the shelf's own `Esc` (close Peek) and the
  // page's (back) listen on the window, which this event does not reach.
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.code !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    onClose();
  };

  return (
    <div
      style={{ display: 'flex', flexDirection: 'column', gap: 12 }}
      onKeyDown={onKeyDown}
      data-testid="sum-panel"
    >
      <button
        ref={closeRef}
        type="button"
        style={{ ...outlineButton, alignSelf: 'flex-end' }}
        onClick={onClose}
      >
        {CLOSE_LABEL}
      </button>
      {clauses === null ? (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 9 }}>
          <h2 style={emptyHeading}>{NO_SCAN_YET_HEADING}</h2>
          <p style={detailStyle}>{NO_SCAN_YET_BODY}</p>
        </div>
      ) : (
        <SummaryBody
          clauses={clauses}
          groups={groups}
          problemsDismissed={problemsDismissed}
          request={request}
          onOpenProject={onOpenProject}
          onChanged={onChanged}
        />
      )}
    </div>
  );
}

function SummaryBody(props: {
  readonly clauses: NonNullable<ReturnType<typeof summaryHeaderClauses>>;
  readonly groups: readonly ProblemGroup[];
  readonly problemsDismissed: boolean;
  readonly request: ErrorRequest;
  readonly onOpenProject: (id: ProjectId) => void;
  readonly onChanged: () => void;
}): ReactElement {
  const { clauses, groups, problemsDismissed, request, onOpenProject, onChanged } = props;
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

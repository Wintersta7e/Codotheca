/**
 * §8.5.5. The chart's DOM is deliberately incapable of reading as a stack: a slot is a flex row
 * of two sub-columns, each ending on the slot's own floor, with no wrapper accumulating height
 * between them. §9's "never summed" is a geometric instruction here, not only an arithmetic one.
 */
import type { ReactElement } from 'react';

import type { ProjectDetail } from '../../../generated/protocol';
import { appearanceFor, fadeFor, jewelAlpha, seedOf } from '../../art/appearance';
import {
  AXIS_LABELS,
  commitAlpha,
  commitCell,
  formatCommitDate,
  ledgerNote,
  LEGEND,
  sessionCell,
  weekTooltip,
  WEEK_SLOTS,
  type LaneCell,
} from './activityLanes';

export const RECENT_COMMITS_LABEL = 'RECENT COMMITS';

export interface ActivityTabProps {
  detail: ProjectDetail;
}

function laneNode(cell: LaneCell, lane: string, fill: string): ReactElement {
  if (cell.kind === 'blank') {
    return <div className="cp-act-lane" data-lane={lane} data-cell="blank" />;
  }
  if (cell.kind === 'hairline') {
    return (
      <div className="cp-act-lane cp-act-lane-hairline" data-lane={lane} data-cell="hairline" />
    );
  }
  return (
    <div
      className="cp-act-lane"
      data-lane={lane}
      data-cell="bar"
      style={{ height: `${String(cell.heightPct)}%`, background: fill }}
    />
  );
}

export function ActivityTab({ detail }: ActivityTabProps): ReactElement {
  // §7.3a's derivation, through the function the card already uses, so the commit lane is this
  // project's colour on this page exactly as it is on its tile.
  const appearance = appearanceFor(
    seedOf(detail.row),
    fadeFor(detail.row),
    detail.row.primaryLanguage,
  );
  const activity = detail.activity;
  const weeks = activity.weeks.slice(-WEEK_SLOTS);

  return (
    <section className="cp-act">
      <h2 className="cp-act-title">ACTIVITY</h2>
      <div className="cp-act-note" data-testid="cp-act-note">
        {ledgerNote(activity)}
      </div>

      <div className="cp-act-chart" role="img" aria-label={ledgerNote(activity)}>
        {weeks.map((week, i) => (
          <div
            key={`${String(week.weekStart)}-${String(i)}`}
            className="cp-act-slot"
            data-testid="cp-act-slot"
            title={weekTooltip(week, activity)}
          >
            {laneNode(
              commitCell(week, activity.commitDays),
              'commitDays',
              jewelAlpha(appearance, commitAlpha(week.commitDays ?? 0)),
            )}
            {laneNode(sessionCell(week, activity.sessions), 'sessions', 'var(--sig)')}
          </div>
        ))}
      </div>

      <div className="cp-act-axis" data-testid="cp-act-axis">
        <span>{AXIS_LABELS[0]}</span>
        <span>{AXIS_LABELS[1]}</span>
      </div>

      <div className="cp-act-legend">
        {LEGEND.map((entry) => (
          <span key={entry.id} className="cp-act-legend-item">
            <span
              className="cp-act-swatch"
              aria-hidden="true"
              style={{
                background: entry.id === 'commitDays' ? jewelAlpha(appearance, 0.95) : 'var(--sig)',
              }}
            />
            <span data-testid="cp-act-legend-label">{entry.label}</span>
          </span>
        ))}
      </div>

      {detail.recentCommits.length === 0 ? null : (
        <>
          <h2 className="cp-act-commits-label" data-testid="cp-act-commits-label">
            {RECENT_COMMITS_LABEL}
          </h2>
          <ul className="cp-act-commits">
            {detail.recentCommits.map((commit) => (
              <li key={commit.sha} className="cp-act-commit">
                <span className="cp-act-commit-sha" data-testid="cp-act-commit-sha">
                  {commit.sha.slice(0, 7)}
                </span>
                <span className="cp-act-commit-subject" data-testid="cp-act-commit-subject">
                  {commit.subject}
                </span>
                <span className="cp-act-commit-date" data-testid="cp-act-commit-date">
                  {formatCommitDate(commit.at, commit.tzOffsetMin)}
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}

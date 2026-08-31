import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { activityFixture, commitFixture, DAY, detailFixture, NOW } from '../testFixtures';
import { ActivityTab, RECENT_COMMITS_LABEL } from './ActivityTab';

afterEach(cleanup);

const draw = (over: Parameters<typeof detailFixture>[0] = {}): HTMLElement =>
  render(<ActivityTab detail={detailFixture(over)} />).container;

describe('the chart', () => {
  /**
   * Criterion 40. `querySelectorAll` is a *descendant* search, so asserting through it passes
   * against a wrapper stacking one lane over the other — measured: adding `<div>` around both
   * lanes left every assertion green. The lanes must therefore be the slot's own children, and
   * the slot must have exactly two of them, so no element can accumulate height between them.
   */
  it('draws 26 slots, each holding two adjacent lanes and no stack', () => {
    const container = draw();
    const slots = container.querySelectorAll('[data-testid="cp-act-slot"]');
    expect(slots).toHaveLength(26);
    for (const slot of slots) {
      const lanes = [...slot.children];
      expect(lanes).toHaveLength(2);
      expect(lanes.map((l) => l.getAttribute('data-lane'))).toEqual(['commitDays', 'sessions']);
      // Adjacent, not nested: neither lane may contain the other, and neither has children.
      expect(lanes[0]?.contains(lanes[1] as Node)).toBe(false);
      expect(lanes[1]?.contains(lanes[0] as Node)).toBe(false);
      expect(lanes[0]?.children).toHaveLength(0);
      expect(lanes[1]?.children).toHaveLength(0);
    }
    // Each lane ends on the slot's own floor rather than on the other lane's top. That half is
    // in the stylesheet, so it is asserted where the stylesheet is loaded: styles/projectPage.
  });

  it('leaves the session lane empty on a project this install has never launched', () => {
    const container = draw();
    const sessions = container.querySelectorAll('[data-lane="sessions"][data-cell="bar"]');
    expect(sessions).toHaveLength(0);
    expect(
      container.querySelectorAll('[data-lane="sessions"][data-cell="hairline"]').length,
    ).toBeGreaterThan(0);
  });

  it('draws no hairline at all for a lane nothing has computed', () => {
    const container = draw({ activity: activityFixture({ commitDays: 'not_computed' }) });
    expect(container.querySelectorAll('[data-lane="commitDays"][data-cell="bar"]')).toHaveLength(0);
    expect(
      container.querySelectorAll('[data-lane="commitDays"][data-cell="hairline"]'),
    ).toHaveLength(0);
  });

  it('states which lane is blank and why', () => {
    draw({ activity: activityFixture({ commitDays: 'shallow_excluded' }) });
    expect(screen.getByTestId('cp-act-note').textContent).toBe(
      'HISTORY IS SHALLOW — COMMIT-DAYS NOT COUNTED · NO SESSIONS YET',
    );
  });

  it('names the two ledgers in the legend and the axis at both ends', () => {
    draw();
    expect(screen.getAllByTestId('cp-act-legend-label').map((n) => n.textContent)).toEqual([
      'COMMIT-DAYS',
      'LAUNCHED SESSIONS',
    ]);
    expect(screen.getByTestId('cp-act-axis').textContent).toContain('26 WEEKS AGO');
    expect(screen.getByTestId('cp-act-axis').textContent).toContain('THIS WEEK');
  });

  it('carries a tooltip naming both ledgers and no sum', () => {
    const container = draw({
      activity: activityFixture({
        weeks: Array.from({ length: 26 }, () => ({
          weekStart: NOW,
          commitDays: 4,
          sessionCount: 2,
          sessionSeconds: 60,
        })),
      }),
    });
    const slot = container.querySelector('[data-testid="cp-act-slot"]');
    expect(slot?.getAttribute('title')).toBe('4 commit-days · 2 sessions');
  });

  it('paints the commit lane in this project’s own jewel and the session lane in the signature', () => {
    const container = draw();
    const bar = container.querySelector('[data-lane="commitDays"][data-cell="bar"]');
    expect(bar?.getAttribute('style')).toMatch(/oklch\(/);
    const swatches = container.querySelectorAll('.cp-act-swatch');
    expect(swatches[1]?.getAttribute('style')).toContain('var(--sig)');
  });

  /**
   * `RECENT COMMITS` is §8.5.5's own heading, so the ban is on a commit **count** and on the
   * superseded legend, not on the word. J4 produces days and nothing in the schema stores a
   * count; a figure like `9 commits` is the shape never-reward-volume forbids.
   */
  it('renders no commit count and no volume figure anywhere', () => {
    const container = draw();
    const text = container.textContent ?? '';
    const titles = [...container.querySelectorAll('[title]')]
      .map((n) => n.getAttribute('title'))
      .join(' ');
    for (const surface of [text, titles, container.innerHTML]) {
      expect(surface).not.toMatch(/COMMITS BY YOU/i);
      expect(surface).not.toMatch(/\d+\s+commits\b/i);
      expect(surface).not.toMatch(/lines of code|\bLOC\b/i);
    }
  });
});

describe('RECENT COMMITS', () => {
  it('lists sha, subject and a date in the commit’s own zone', () => {
    draw({
      recentCommits: [
        commitFixture({
          sha: 'abc123def456',
          subject: 'a shaped subject',
          at: NOW - DAY,
          tzOffsetMin: 0,
        }),
      ],
    });
    expect(screen.getByTestId('cp-act-commits-label').textContent).toBe(RECENT_COMMITS_LABEL);
    expect(screen.getByTestId('cp-act-commit-sha').textContent).toBe('abc123d');
    expect(screen.getByTestId('cp-act-commit-subject').textContent).toBe('a shaped subject');
    expect(screen.getByTestId('cp-act-commit-date').textContent).toBe('2027-01-14');
  });

  it('draws no list at all rather than an empty one', () => {
    draw({ recentCommits: [] });
    expect(screen.queryByTestId('cp-act-commits-label')).toBeNull();
  });
});

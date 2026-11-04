/**
 * §43.13's Profile contents: the level's evidence sums to its XP, the floor says it holds, the
 * badges are *not computed* or not drawn by whether history was read, opening Profile writes
 * nothing, and under quiet nothing quiet mode lists is on the page.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, test, vi } from 'vitest';

import type { BadgeAward, BadgesListed, MotivationState } from '../../generated/protocol';
import { DATA_MOTIVATION } from '../../shared/motivationQuiet';
import { required } from '../../shared/required';
import type { AppDeps } from '../app/deps';
import type { ProfileAnchor } from '../app/route';
import { badgeAwardCopy } from '../badgeAward/copy';
import { FLOOR_HOLDS_LINE, LEVEL_NOT_COMPUTED, XP_TERM_WORDS } from '../motivation/copy';
import { ProfileBadgesSection, ProfileLevelSection } from './ProfileMotivation';
import { ProfileScreen } from './ProfileScreen';

afterEach(() => {
  cleanup();
});

const state = (over: Partial<MotivationState> = {}): MotivationState => ({
  shown: true,
  level: {
    state: 'computed',
    figures: { level: 6, xp: 935, levelAt: 800, nextLevelAt: 1_100, floored: false },
    pendingProjects: 0,
  },
  rings: null,
  xpBar: 0.45,
  pendingLine: null,
  terms: [
    { term: 'commitDay', xp: 600 },
    { term: 'release', xp: 80 },
    { term: 'focus', xp: 105 },
    { term: 'revival', xp: 150 },
  ],
  ...over,
});

const award = (kind: BadgeAward['kind'], earnedOn: string): BadgeAward => ({
  kind,
  earnedOn,
  projectId: null,
  evidence: {
    projectName: null,
    gapDays: null,
    year: null,
    languages: null,
    creditedSeconds: null,
    projects: null,
    runStartOn: null,
  },
  landed: true,
});

const listed = (awards: BadgeAward[], historyComplete: boolean): BadgesListed => ({
  awards,
  due: null,
  basis: { projectsCovered: 2, projectsTotal: 2, historyComplete },
});

const twoAwards = listed([award('polyglot', '2026-10-01'), award('curator', '2026-10-20')], true);

/** A `request` that answers `badges.list` with `answer` and records every call. */
function fakeRequest(answer: BadgesListed): {
  request: AppDeps['request'];
  calls: string[];
} {
  const calls: string[] = [];
  const request = vi.fn((name: string) => {
    calls.push(name);
    return Promise.resolve(name === 'badges.list' ? answer : {});
  }) as unknown as AppDeps['request'];
  return { request, calls };
}

async function mountScreen(
  shown: MotivationState | null,
  answer: BadgesListed,
  at: ProfileAnchor = { section: 'level' },
): Promise<{ calls: string[]; onBack: () => void }> {
  const { request, calls } = fakeRequest(answer);
  const onBack = vi.fn();
  render(<ProfileScreen state={shown} at={at} request={request} onBack={onBack} />);
  // Let `badges.list`'s answer land.
  await act(async () => {
    await Promise.resolve();
  });
  return { calls, onBack };
}

const quietCount = (): number => document.querySelectorAll(`[${DATA_MOTIVATION}]`).length;

test("AC-P4-43-17-profile Profile's motivation contents carry data-motivation, and none under quiet", async () => {
  await mountScreen(state(), twoAwards);
  const shownCount = quietCount();
  expect(shownCount).toBeGreaterThan(0);
  expect(document.querySelector(`[${DATA_MOTIVATION}="level"]`)).not.toBeNull();
  expect(document.querySelector(`[${DATA_MOTIVATION}="badges"]`)).not.toBeNull();
  cleanup();

  // Quiet: the core strips its payload at the source, so the state arrives `shown: false`.
  const quiet = state({ shown: false, level: null, xpBar: null, terms: null });
  await mountScreen(quiet, twoAwards);
  const quietTotal = quietCount();
  expect(quietTotal).toBe(0);
  // The page itself is still there, with its way back.
  expect(screen.getByRole('button')).not.toBeNull();
});

describe('the level section', () => {
  test("renders every term, and the terms it renders sum to the level's XP", () => {
    render(<ProfileLevelSection state={state()} />);
    const shown = state();
    const rows = [...document.querySelectorAll('[data-term]')];
    expect(rows).toHaveLength(4);
    let total = 0;
    for (const row of rows) {
      const term = row.getAttribute('data-term') as keyof typeof XP_TERM_WORDS;
      expect(row.textContent).toContain(XP_TERM_WORDS[term]);
      total += Number(row.querySelector('[data-xp]')?.textContent);
    }
    expect(total).toBe(shown.level?.figures?.xp);
    expect(Number(document.querySelector('[data-sum]')?.textContent)).toBe(total);
    expect(screen.queryByText(FLOOR_HOLDS_LINE)).toBeNull();
  });

  test('says the floor holds the number when the level is floored', () => {
    const floored = state({
      level: {
        state: 'computed',
        figures: { level: 6, xp: 700, levelAt: 800, nextLevelAt: 1_100, floored: true },
        pendingProjects: 0,
      },
      terms: [{ term: 'commitDay', xp: 700 }],
    });
    render(<ProfileLevelSection state={floored} />);
    expect(screen.getByText(FLOOR_HOLDS_LINE)).not.toBeNull();
  });

  test('has nothing to sum when the level carries no figures, and draws no zero', () => {
    const unknown = state({
      level: { state: 'notComputed', figures: null, pendingProjects: 4 },
      xpBar: null,
      terms: null,
    });
    render(<ProfileLevelSection state={unknown} />);
    expect(document.querySelector('[data-term]')).toBeNull();
    expect(document.querySelector('[data-sum]')).toBeNull();
    expect(document.body.textContent).toContain('—');
    expect(document.body.textContent).not.toMatch(/\b0\b/u);
  });
});

describe('the badges section', () => {
  test('history unread and none latched is not computed; history read and none is not drawn', () => {
    const unread = render(<ProfileBadgesSection listed={listed([], false)} shown />);
    expect(unread.container.textContent).toContain(LEVEL_NOT_COMPUTED);
    cleanup();
    const read = render(<ProfileBadgesSection listed={listed([], true)} shown />);
    expect(read.container.innerHTML).toBe('');
  });

  test("lists each award in badges.list's order, dated, with its evidence sentence", () => {
    render(<ProfileBadgesSection listed={twoAwards} shown />);
    const items = [...document.querySelectorAll('li')];
    expect(items.map((item) => item.querySelector('time')?.getAttribute('dateTime'))).toEqual([
      '2026-10-01',
      '2026-10-20',
    ]);
    for (const [index, item] of items.entries()) {
      const copy = badgeAwardCopy(required(twoAwards.awards[index], 'listed award'));
      expect(item.textContent).toContain(copy.name);
      expect(item.textContent).toContain(copy.sentence);
      // Never a slug, a count or a meter.
      expect(item.textContent).not.toContain(
        required(twoAwards.awards[index], 'listed award').kind,
      );
      expect(item.querySelector('progress, meter')).toBeNull();
    }
  });

  test('draws nothing under quiet, and nothing before the list is read', () => {
    expect(
      render(<ProfileBadgesSection listed={twoAwards} shown={false} />).container.innerHTML,
    ).toBe('');
    expect(render(<ProfileBadgesSection listed={null} shown />).container.innerHTML).toBe('');
  });
});

describe('the Profile screen', () => {
  test('opening it reads badges.list and writes nothing — no acknowledgement, no landing', async () => {
    const withLine = state({
      pendingLine: {
        form: 'badge',
        level: null,
        reason: null,
        award: { ...award('curator', '2026-10-20'), landed: false },
      },
    });
    const { calls } = await mountScreen(withLine, twoAwards, {
      section: 'badges',
      badge: 'curator',
    });
    expect(calls).toEqual(['badges.list']);
    // §43.4: the line is nowhere but the band — Profile does not draw it.
    expect(document.querySelector(`[${DATA_MOTIVATION}="line"]`)).toBeNull();
  });

  test('Esc returns to the shelf, and so does its way back', async () => {
    const { onBack } = await mountScreen(state(), twoAwards);
    const root = document.querySelector('.cdt-profile');
    expect(document.activeElement).toBe(root);
    fireEvent.keyDown(required(root, 'Profile root'), { key: 'Escape' });
    expect(onBack).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button'));
    expect(onBack).toHaveBeenCalledTimes(2);
  });
});

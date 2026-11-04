/**
 * §43.4's line, renderer half: each form's copy, the badge's landing on paint and never on click,
 * the click's acknowledgement and where it opens Profile, and that the line waits to be found —
 * no live region, no alert.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, test, vi } from 'vitest';

import type {
  BadgeAward,
  LevelReading,
  MotivationLine,
  MotivationReason,
} from '../../generated/protocol';
import { DATA_MOTIVATION } from '../../shared/motivationQuiet';
import type { AppDeps } from '../app/deps';
import { badgeAwardCopy } from '../badgeAward/copy';
import { LINE_ARRIVAL, LINE_COVERAGE_PARTIAL, LINE_RETURN, LINE_RISE } from './copy';
import { lineModel } from './line';
import { MotivationLineButton } from './MotivationLineButton';

afterEach(() => {
  cleanup();
});

const computed: LevelReading = {
  state: 'computed',
  figures: { level: 6, xp: 900, levelAt: 800, nextLevelAt: 1_100, floored: false },
  pendingProjects: 0,
};
const partial: LevelReading = { ...computed, state: 'partial', pendingProjects: 3 };

const reason: MotivationReason = {
  transitions: 1,
  term: 'release',
  projectName: 'alpha',
  on: '2026-11-02',
};

const award = (over: Partial<BadgeAward> = {}): BadgeAward => ({
  kind: 'curator',
  earnedOn: '2026-11-01',
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
  landed: false,
  ...over,
});

const line = (over: Partial<MotivationLine>): MotivationLine => ({
  form: 'rise',
  level: 6,
  reason: null,
  award: null,
  ...over,
});

const badgeLine = (over: Partial<BadgeAward> = {}): MotivationLine =>
  line({ form: 'badge', level: null, award: award(over) });

/** A `request` that answers every call and records it. */
function fakeRequest(): { request: AppDeps['request']; calls: [string, unknown][] } {
  const calls: [string, unknown][] = [];
  const request = vi.fn((name: string, args: unknown) => {
    calls.push([name, args]);
    return Promise.resolve({});
  }) as unknown as AppDeps['request'];
  return { request, calls };
}

function mount(
  shown: MotivationLine,
  onScreen: boolean,
  level: LevelReading | null = computed,
): {
  calls: [string, unknown][];
  opened: unknown[];
  rerender: (onScreen: boolean, next?: MotivationLine) => void;
} {
  const { request, calls } = fakeRequest();
  const opened: unknown[] = [];
  const onOpenProfile = (at: unknown): void => {
    opened.push(at);
  };
  const view = render(
    <MotivationLineButton
      line={shown}
      level={level}
      onScreen={onScreen}
      request={request}
      onOpenProfile={onOpenProfile}
    />,
  );
  return {
    calls,
    opened,
    rerender: (next: boolean, nextLine = shown) => {
      view.rerender(
        <MotivationLineButton
          line={nextLine}
          level={level}
          onScreen={next}
          request={request}
          onOpenProfile={onOpenProfile}
        />,
      );
    },
  };
}

const named = (calls: [string, unknown][], name: string): unknown[] =>
  calls.filter(([called]) => called === name).map(([, args]) => args);

test('AC-P4-43-7-renderer each form renders its own copy and a rise with no reason renders the level alone', () => {
  const forms: [MotivationLine, LevelReading, string][] = [
    [line({ form: 'arrival' }), computed, `${LINE_ARRIVAL} 6`],
    [line({ form: 'arrival' }), partial, `${LINE_ARRIVAL} 6 · ${LINE_COVERAGE_PARTIAL(3)}`],
    [line({ form: 'rise', reason }), computed, LINE_RISE(6, reason)],
    [line({ form: 'rise', reason: null }), computed, LINE_RISE(6, null)],
    [line({ form: 'return' }), computed, `${LINE_RETURN} 6`],
  ];
  for (const [shown, level, expected] of forms) {
    expect(lineModel(shown, level).text).toBe(expected);
    mount(shown, false, level);
    expect(screen.getByRole('button').textContent).toBe(expected);
    cleanup();
  }
  // A rise with a reason names its receipt; with none it is the level and nothing after it.
  expect(LINE_RISE(6, reason)).toContain('alpha');
  expect(LINE_RISE(6, null).endsWith(' 6')).toBe(true);
  expect(LINE_RISE(6, null)).not.toContain('·');

  // A badge line is §40's name and evidence sentence, read from §40's one copy module.
  const copy = badgeAwardCopy(award());
  mount(badgeLine(), false);
  const text = screen.getByRole('button').textContent;
  expect(text).toContain(copy.name);
  expect(text).toContain(copy.sentence);
});

test('AC-P4-43-6-renderer a badge lands when its line is painted, never on click, and a click opens Profile', () => {
  // Painted in an active window: the landing goes once, and a re-render sends nothing more.
  const painted = mount(badgeLine(), true);
  painted.rerender(true);
  expect(named(painted.calls, 'badges.markLanded')).toEqual([{ kind: 'curator' }]);
  // The click acknowledges and opens Profile at the badge; it lands nothing more.
  fireEvent.click(screen.getByRole('button'));
  expect(named(painted.calls, 'badges.markLanded')).toHaveLength(1);
  expect(named(painted.calls, 'motivation.acknowledge')).toEqual([
    { event: 'badgeLineClicked', level: null },
  ]);
  expect(painted.opened).toEqual([{ section: 'badges', badge: 'curator' }]);
  cleanup();

  // Hidden or unfocused: nothing is painted, so nothing lands — and a click still lands nothing.
  const hidden = mount(badgeLine(), false);
  fireEvent.click(screen.getByRole('button'));
  expect(named(hidden.calls, 'badges.markLanded')).toEqual([]);
  // It lands the first time it comes on screen.
  hidden.rerender(true);
  expect(named(hidden.calls, 'badges.markLanded')).toEqual([{ kind: 'curator' }]);
  cleanup();

  // An award already landed is painted and lands nothing.
  const landed = mount(badgeLine({ landed: true }), true);
  expect(named(landed.calls, 'badges.markLanded')).toEqual([]);
  cleanup();

  // A level line's click sends the level it shows and opens Profile at the level.
  const level = mount(line({ form: 'rise', level: 6, reason }), true);
  fireEvent.click(screen.getByRole('button'));
  expect(named(level.calls, 'motivation.acknowledge')).toEqual([
    { event: 'levelLineClicked', level: 6 },
  ]);
  expect(named(level.calls, 'badges.markLanded')).toEqual([]);
  expect(level.opened).toEqual([{ section: 'level' }]);

  // It waits to be found: no live region and no alert, on the line or inside it.
  const button = screen.getByRole('button');
  expect(button.getAttribute(DATA_MOTIVATION)).toBe('line');
  for (const element of [button, ...button.querySelectorAll('*')]) {
    expect(element.hasAttribute('aria-live')).toBe(false);
    expect(element.getAttribute('role')).not.toBe('alert');
    expect(element.getAttribute('role')).not.toBe('status');
  }
});

describe('the line', () => {
  test('an arrival and a return open Profile at the level with the level they show', () => {
    for (const form of ['arrival', 'return'] as const) {
      const shown = mount(line({ form, level: 9 }), true);
      fireEvent.click(screen.getByRole('button'));
      expect(named(shown.calls, 'motivation.acknowledge')).toEqual([
        { event: 'levelLineClicked', level: 9 },
      ]);
      expect(shown.opened).toEqual([{ section: 'level' }]);
      cleanup();
    }
  });

  test('a later award of the same kind lands again once its line is painted', () => {
    const shown = mount(badgeLine(), true);
    shown.rerender(true, badgeLine({ earnedOn: '2026-12-01' }));
    expect(named(shown.calls, 'badges.markLanded')).toEqual([
      { kind: 'curator' },
      { kind: 'curator' },
    ]);
  });
});

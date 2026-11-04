import { expect, test } from 'vitest';

import type {
  BadgeAward,
  BadgesListed,
  LevelReading,
  MotivationState,
  ProjectId,
  Reveal,
  RevealBasis,
} from '../../generated/protocol';
import { badgeAwardCopy } from '../badgeAward/copy';
import { FLOOR_HOLDS_LINE, TERMS_SUM_LABEL, XP_TERM_WORDS } from '../motivation/copy';
import { COVERAGE_HISTORY_GROWS } from './copy';
import { PANEL_ENTRY_FIRST_SEC, PANEL_ENTRY_STEP_SEC, revealPanels } from './revealModel';
import {
  badgePanel,
  badgePending,
  levelPanel,
  revealGridFloorPx,
  revealSequence,
} from './revealMotivation';

const full: RevealBasis = { projectsCovered: 4, projectsTotal: 4, historyComplete: true };
const unread: RevealBasis = { ...full, historyComplete: false };

const reveal: Reveal = {
  spanDays: { value: 4_400, basis: full },
  projectCount: { value: 4, basis: full },
  languageCount: { value: 2, basis: full },
  bestYear: { value: 2021, basis: full },
  playtimeSeconds: { value: 0, basis: full },
  oldestStillAlive: { projectId: 1 as ProjectId, firstCommitAt: 1, basis: full },
};

const six = revealPanels(reveal, {
  nowSecs: 1_760_000_000,
  languageTally: [],
  referenceCount: 0,
  project: () => null,
});

const reading = (
  state: LevelReading['state'],
  level: number | null,
  floored = false,
): LevelReading => ({
  state,
  figures: level === null ? null : { level, xp: 935, levelAt: 800, nextLevelAt: 1_100, floored },
  pendingProjects: state === 'partial' ? 3 : 0,
});

const motivation = (over: Partial<MotivationState> = {}): MotivationState => ({
  shown: true,
  level: reading('computed', 6),
  rings: null,
  xpBar: 0.45,
  pendingLine: null,
  terms: [
    { term: 'commitDay', xp: 600 },
    { term: 'release', xp: 335 },
  ],
  ...over,
});

const award = (landed = false): BadgeAward => ({
  kind: 'polyglot',
  earnedOn: '2021-11-04',
  projectId: null,
  evidence: {
    projectName: null,
    gapDays: null,
    year: 2021,
    languages: ['Rust', 'Go', 'C', 'Lua', 'Zig'],
    creditedSeconds: null,
    projects: null,
    runStartOn: null,
  },
  landed,
});

const listed = (due: boolean, basis: RevealBasis, landed = false): BadgesListed => ({
  awards: [award(landed)],
  due: due ? 'polyglot' : null,
  basis,
});

const labels = (panels: readonly { label: string }[]): string[] => panels.map((p) => p.label);

test('AC-P4-43-11 the reveal holds eight panels in order, gold on the eighth only, and LEVEL is never a zero', () => {
  const eight = revealSequence(six, levelPanel(motivation()), badgePanel(listed(true, full), true));
  expect(labels(eight)).toEqual([
    'SPAN',
    'PROJECTS',
    'LANGUAGES',
    'BEST YEAR',
    'OLDEST STILL ALIVE',
    'LEVEL',
    'PLAYTIME',
    'BADGE EARNED',
  ]);
  expect(eight.map((p) => p.edge)).toEqual(['sig', null, null, null, null, null, null, 'gold']);
  // §10.4a's cascade by position: through 0.55 s across eight.
  eight.forEach((panel, i) => {
    expect(panel.delaySec).toBeCloseTo(PANEL_ENTRY_FIRST_SEC + i * PANEL_ENTRY_STEP_SEC, 10);
  });
  expect(eight.at(-1)?.delaySec).toBeCloseTo(0.55, 10);

  // Never unknown as a number: notComputed, and a reading with no figures, is *not computed*.
  for (const level of [reading('notComputed', null), null, reading('computed', null)]) {
    const panel = levelPanel(motivation({ level }));
    expect(panel?.value).toBeNull();
    expect(panel?.shownLevel).toBeNull();
    expect(panel?.coverage).toBeNull();
  }

  // BADGE EARNED: not computed while history is unread with nothing due; absent over complete
  // history with nothing due.
  const pending = badgePanel(listed(false, unread), true);
  expect(pending?.value).toBeNull();
  expect(pending?.toLand).toBeNull();
  expect(badgePanel(listed(false, full), true)).toBeNull();
  const seven = revealSequence(
    six,
    levelPanel(motivation()),
    badgePanel(listed(false, full), true),
  );
  expect(labels(seven)).not.toContain('BADGE EARNED');
  expect(seven.filter((p) => p.edge === 'gold')).toHaveLength(0);

  // The floor follows the count.
  expect(revealGridFloorPx(6)).toBe(260);
  expect(revealGridFloorPx(7)).toBe(200);
  expect(revealGridFloorPx(8)).toBe(200);

  // Under quiet both are absent and the reveal is §10.4a's six.
  const quiet = motivation({ shown: false, level: null, terms: null, xpBar: null });
  expect(levelPanel(quiet)).toBeNull();
  expect(badgePanel(listed(true, full), false)).toBeNull();
  expect(labels(revealSequence(six, levelPanel(quiet), null))).toEqual(labels(six));
});

// §43.7: a partial level is true under §10.4a's growth line — added history only adds rows and
// the displayed level never falls. A computed level is complete by construction.
test('a partial level carries the growth line and a computed one carries none', () => {
  const partial = levelPanel(motivation({ level: reading('partial', 4) }));
  expect(partial?.value).toBe('4');
  expect(partial?.shownLevel).toBe(4);
  expect(partial?.coverage).toBe(COVERAGE_HISTORY_GROWS);
  const computed = levelPanel(motivation());
  expect(computed?.value).toBe('6');
  expect(computed?.coverage).toBeNull();
});

// §43.7: SHOW WORKING opens the terms and, when floored, the line saying the floor holds it.
test('the level evidence is the terms, their sum, and the floor line only when floored', () => {
  const plain = levelPanel(motivation())?.evidence ?? '';
  expect(plain).toContain(`${XP_TERM_WORDS.commitDay}  600`);
  expect(plain).toContain(`${XP_TERM_WORDS.release}  335`);
  expect(plain).toContain(`${TERMS_SUM_LABEL}  935`);
  expect(plain).not.toContain(FLOOR_HOLDS_LINE);
  const floored = levelPanel(motivation({ level: reading('computed', 6, true) }))?.evidence;
  expect(floored).toContain(FLOOR_HOLDS_LINE);
});

// §40: a latched badge is a fact, not a measurement — the award's name and sentence, no coverage
// row, and it lands once displayed unless it already has.
test('a due award draws its name and sentence and lands only while unlanded', () => {
  const panel = badgePanel(listed(true, unread), true);
  const copy = badgeAwardCopy(award());
  expect(panel?.value).toBe(copy.name);
  expect(panel?.caption).toBe(copy.sentence);
  expect(panel?.coverage).toBeNull();
  expect(panel?.toLand).toBe('polyglot');
  expect(badgePanel(listed(true, full, true), true)?.toLand).toBeNull();
  expect(badgePending(listed(true, unread))).toBe(false);
  expect(badgePending(listed(false, unread))).toBe(true);
});

// `due` names a kind; a kind the awards do not hold is no answer, so it stays not computed.
test('a due kind with no award behind it is not computed, not an award', () => {
  const orphan: BadgesListed = { awards: [], due: 'comeback', basis: full };
  expect(badgePanel(orphan, true)?.value).toBeNull();
  expect(badgePending(orphan)).toBe(true);
});

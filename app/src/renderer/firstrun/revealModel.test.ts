import { expect, test } from 'vitest';
import {
  MILESTONE_FIGURES,
  PANEL_ENTRY_FIRST_SEC,
  PANEL_ENTRY_STEP_SEC,
  REVEAL_ORDER,
  coverageLine,
  panelFor,
  revealPanels,
  spanYears,
} from './revealModel';
import * as copy from './copy';
import type { RevealDeps } from './revealModel';
import type { ProjectId, Reveal, RevealBasis } from '../../generated/protocol';

const NOW = 1_760_000_000;
const pid = (n: number): ProjectId => n as ProjectId;
const full: RevealBasis = { projectsCovered: 212, projectsTotal: 212, historyComplete: true };
const partial: RevealBasis = { projectsCovered: 212, projectsTotal: 400, historyComplete: false };

const reveal = (basis: RevealBasis = full): Reveal => ({
  spanDays: { value: 4_400, basis },
  projectCount: { value: 212, basis },
  languageCount: { value: 9, basis },
  bestYear: { value: 2021, basis },
  playtimeSeconds: { value: 0, basis: full },
  oldestStillAlive: { projectId: pid(7), firstCommitAt: NOW - 4_400 * 86_400, basis },
});

const deps: RevealDeps = {
  nowSecs: NOW,
  languageTally: [
    { name: 'Rust', count: 12 },
    { name: 'TypeScript', count: 9 },
  ],
  referenceCount: 3,
  project: (id) =>
    id === pid(7) ? { name: 'ledger', birthYear: 2014, primaryLanguage: 'Rust' } : null,
};

// §10.4a: the design's eight panels minus LEVEL and BADGE EARNED are exactly §10.4's 1–6.
test('the panel set is six, in order, with the gold edge dead', () => {
  expect(REVEAL_ORDER).toEqual([
    'spanDays',
    'projectCount',
    'languageCount',
    'bestYear',
    'oldestStillAlive',
    'playtimeSeconds',
  ]);
  const panels = revealPanels(reveal(), deps);
  expect(panels.map((p) => p.label)).toEqual([
    'SPAN',
    'PROJECTS',
    'LANGUAGES',
    'BEST YEAR',
    'OLDEST STILL ALIVE',
    'PLAYTIME',
  ]);
  expect(panels.filter((p) => p.signalEdge).map((p) => p.key)).toEqual(['spanDays']);
});

// §10.4a: delayed 0.06 + i × 0.07 seconds — 0.06s through 0.41s across six. That 70 ms step is
// the reveal's own, not the 80 ms generic panel cascade.
test('the cascade is the reveal own step and lands on 0.41s', () => {
  expect(PANEL_ENTRY_FIRST_SEC).toBe(0.06);
  expect(PANEL_ENTRY_STEP_SEC).toBe(0.07);
  const delays = revealPanels(reveal(), deps).map((p) => Number(p.delaySec.toFixed(2)));
  expect(delays).toEqual([0.06, 0.13, 0.2, 0.27, 0.34, 0.41]);
});

// Criterion 23 and §10.4: at full coverage the line is bare.
test('at full coverage no panel carries a coverage row', () => {
  for (const panel of revealPanels(reveal(), deps)) expect(panel.coverage).toBeNull();
});

// Criterion 23: every reveal figure carries its coverage. There is no path through this that
// leaves a partial figure bare.
test('below full coverage every figure but PLAYTIME says so', () => {
  const panels = revealPanels(reveal(partial), deps);
  for (const panel of panels) {
    if (panel.key === 'playtimeSeconds') {
      expect(panel.coverage).toBeNull();
    } else {
      expect(panel.coverage, panel.label).toContain('across the 212 projects indexed so far');
    }
  }
});

// §10.4a: historyComplete needs its own string, because the coverage string implies growth
// while a history still arriving can move BEST YEAR to a different year.
test('the two honesty strings say different things and join with a middot', () => {
  expect(coverageLine('spanDays', partial)).toBe(
    `${copy.COVERAGE_PARTIAL(212)} · ${copy.COVERAGE_HISTORY_GROWS}`,
  );
  expect(coverageLine('bestYear', partial)).toBe(
    `${copy.COVERAGE_PARTIAL(212)} · ${copy.COVERAGE_HISTORY_MOVES}`,
  );
  expect(coverageLine('oldestStillAlive', partial)).toContain(copy.COVERAGE_HISTORY_MOVES);
  // PROJECTS and LANGUAGES do not move with history, only with coverage.
  expect(coverageLine('projectCount', partial)).toBe(copy.COVERAGE_PARTIAL(212));
  // §10.4a: 0h is complete by construction, and it is the only figure on the screen that is.
  expect(coverageLine('playtimeSeconds', partial)).toBeNull();
  // A history that is incomplete at full coverage still gets its own string.
  expect(
    coverageLine('spanDays', { projectsCovered: 5, projectsTotal: 5, historyComplete: false }),
  ).toBe(copy.COVERAGE_HISTORY_GROWS);
  // And the mirror case, which is the one a plausible implementation loses: partial coverage
  // with a *complete* history. The two conditions are independent, so a coverage string emitted
  // only alongside the history string leaves this figure bare — criterion 23's exact failure.
  const covered: RevealBasis = { projectsCovered: 212, projectsTotal: 400, historyComplete: true };
  expect(coverageLine('projectCount', covered)).toBe(copy.COVERAGE_PARTIAL(212));
  expect(coverageLine('spanDays', covered)).toBe(copy.COVERAGE_PARTIAL(212));
  expect(coverageLine('bestYear', covered)).toBe(copy.COVERAGE_PARTIAL(212));
});

// §10.4a: 22px for OLDEST STILL ALIVE, and its evidence stops at the language.
test('the oldest panel names a project and stops', () => {
  const panel = panelFor('oldestStillAlive', reveal(), deps);
  expect(panel.value).toBe('ledger');
  expect(panel.wide).toBe(true);
  expect(panel.caption).toBe('Still yours.');
  expect(panel.caption).not.toContain('Still builds');
  expect(panel.evidence).toBe('ledger  ·  born 2014  ·  Rust');
  expect(panel.evidence).not.toMatch(/complete|private|public/);
});

// Never render unknown as zero: a figure the core could not compute renders as not computed.
test('an uncomputed figure is named, not zeroed', () => {
  const bare: Reveal = { ...reveal(), bestYear: { value: null, basis: partial } };
  const panel = panelFor('bestYear', bare, deps);
  expect(panel.value).toBeNull();
  expect(panel.coverage).not.toBeNull();
  const oldest = panelFor(
    'oldestStillAlive',
    { ...reveal(), oldestStillAlive: { projectId: null, firstCommitAt: null, basis: partial } },
    deps,
  );
  expect(oldest.value).toBeNull();
});

// §10.4: `Playtime 0h — starts now`. A real zero, computed, and the caption says why.
test('playtime is a real zero and says what it cannot know', () => {
  const panel = panelFor('playtimeSeconds', reveal(), deps);
  expect(panel.value).toBe('0h');
  expect(panel.caption).toContain('Starts now');
  expect(panel.evidence).toContain('Launched-session time only');
});

// Global constraint: never reward volume. No figure for lines or commit counts anywhere.
test('no panel value, caption or evidence names a commit count or a line count', () => {
  for (const panel of revealPanels(reveal(partial), deps)) {
    const text = [panel.value ?? '', panel.caption, panel.evidence].join(' ');
    expect(text).not.toMatch(/\b\d+\s+(commits?|lines?)\b/i);
  }
});

test('span is whole years, and a sub-year library is not reported as zero years', () => {
  expect(spanYears(null)).toBeNull();
  expect(spanYears(4_400)).toBe(12);
  expect(spanYears(365.2425)).toBe(1);
  // GAP-16b-3: §10.4a's ladder has no sub-year rung, so this is null rather than `0 YEARS`.
  expect(spanYears(200)).toBeNull();
});

// §10.3a: a milestone figure and §10.4's reveal share one basis; if they disagree, the basis is
// wrong. The only structural guarantee is that a milestone *is* a reveal panel.
test('every milestone maps to a reveal figure this module computes', () => {
  expect(MILESTONE_FIGURES).toEqual({
    10: 'languageCount',
    50: 'spanDays',
    100: 'oldestStillAlive',
    250: 'bestYear',
    500: 'spanDays',
  });
  for (const key of Object.values(MILESTONE_FIGURES)) {
    expect(REVEAL_ORDER).toContain(key);
  }
});

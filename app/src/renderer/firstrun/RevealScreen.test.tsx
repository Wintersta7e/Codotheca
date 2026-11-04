import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, test, vi } from 'vitest';
import { RevealScreen } from './RevealScreen';
import type { RevealMotivation } from './RevealScreen';
import * as copy from './copy';
import type { RevealDeps } from './revealModel';
import type {
  BadgeAward,
  BadgesListed,
  MotivationState,
  ProjectId,
  Reveal,
  RevealBasis,
} from '../../generated/protocol';
import { required } from '../../shared/required';
import { BADGE_EARNED_LABEL, LEVEL_LABEL } from '../motivation/copy';

let visibility: DocumentVisibilityState = 'visible';

beforeEach(() => {
  visibility = 'visible';
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => visibility,
  });
  vi.spyOn(document, 'hasFocus').mockReturnValue(true);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

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
  languageTally: [{ name: 'Rust', count: 12 }],
  referenceCount: 3,
  project: () => ({ name: 'ledger', birthYear: 2014, primaryLanguage: 'Rust' }),
};

/** Quiet mode's payload: every field but `shown` stripped at the source. */
const QUIET: MotivationState = {
  shown: false,
  level: null,
  rings: null,
  xpBar: null,
  pendingLine: null,
  terms: null,
};

const shownAt = (
  level: number | null,
  state: 'partial' | 'computed' = 'computed',
): MotivationState => ({
  shown: true,
  level: {
    state: level === null ? 'notComputed' : state,
    figures:
      level === null ? null : { level, xp: 935, levelAt: 800, nextLevelAt: 1_100, floored: false },
    pendingProjects: 0,
  },
  rings: null,
  xpBar: null,
  pendingLine: null,
  terms: [{ term: 'commitDay', xp: 935 }],
});

const polyglot: BadgeAward = {
  kind: 'polyglot',
  earnedOn: '2021-11-04',
  projectId: null,
  evidence: {
    projectName: null,
    gapDays: null,
    year: 2021,
    languages: null,
    creditedSeconds: null,
    projects: null,
    runStartOn: null,
  },
  landed: false,
};

const due: BadgesListed = { awards: [polyglot], due: 'polyglot', basis: full };

/** The defaults are quiet with no badge: §10.4a's six panels, which the phase-1 tests pin. */
function motivationOf(over: Partial<RevealMotivation> = {}): RevealMotivation {
  return {
    state: QUIET,
    badges: null,
    acknowledge: vi.fn(() => Promise.resolve()),
    markLanded: vi.fn(() => Promise.resolve()),
    ...over,
  };
}

function draw(over: Partial<Parameters<typeof RevealScreen>[0]> = {}): {
  onGoOn: ReturnType<typeof vi.fn>;
  rerender: (next: Partial<Parameters<typeof RevealScreen>[0]>) => void;
} {
  const onGoOn = vi.fn();
  const base = {
    reveal: reveal(),
    deps,
    motivation: motivationOf(),
    tier: 'full',
    onGoOn,
  } as const;
  const view = render(<RevealScreen {...base} {...over} />);
  return {
    onGoOn,
    rerender: (next) => {
      view.rerender(<RevealScreen {...base} {...over} {...next} />);
    },
  };
}

// §10.4a: the eyebrow's <n> PROJECTS and the PROJECTS panel are one number from one call.
test('the eyebrow and the PROJECTS panel cannot disagree', () => {
  draw();
  expect(screen.getByText(copy.REVEAL_EYEBROW(212))).toBeTruthy();
  const panel = screen.getByRole('button', { name: /PROJECTS/ });
  expect(within(panel).getByTestId('fr-panel-value').textContent).toBe('212');
});

// §10.4a: span is monotone under increasing coverage, so AT LEAST is exactly honest.
test('the headline takes AT LEAST below full coverage and nothing above it', () => {
  draw();
  expect(screen.getByRole('heading').textContent).toBe(copy.revealHeadline(12, true));
  cleanup();
  draw({ reveal: reveal(partial) });
  expect(screen.getByRole('heading').textContent).toBe(copy.revealHeadline(12, false));
});

// Criterion 23: every reveal figure carries its coverage. Six panels, five coverage rows —
// PLAYTIME is complete by construction.
test('below full coverage five of six panels carry a coverage row', () => {
  draw({ reveal: reveal(partial) });
  const rows = screen.getAllByTestId('fr-panel-coverage');
  expect(rows).toHaveLength(5);
  for (const row of rows) expect(row.textContent).toContain('indexed so far');
});

// The case a plausible implementation loses: a partial index whose history has finished. The
// two conditions are independent, so a coverage row drawn only while the history is still
// arriving presents five figures over 212 of 400 projects as if they covered all of them.
test('a partial index with a finished history still carries its coverage', () => {
  const covered: RevealBasis = { projectsCovered: 212, projectsTotal: 400, historyComplete: true };
  draw({ reveal: reveal(covered) });
  const rows = screen.getAllByTestId('fr-panel-coverage');
  expect(rows).toHaveLength(5);
  for (const row of rows) expect(row.textContent).toBe(copy.COVERAGE_PARTIAL(212));
});

test('at full coverage no panel carries a coverage row', () => {
  draw();
  expect(screen.queryAllByTestId('fr-panel-coverage')).toHaveLength(0);
});

// §10.4a: six lay out 3+3 under a 260px floor. Four panels in a row would leave a two-item row
// that reads as two panels that failed to load. This is the quiet, no-badge case drawn
// explicitly: seven and eight panels, and the gold edge, are AC-P4-43-11's and -12's.
test('under quiet with no badge due there are exactly six panels and neither phase-4 panel is drawn', () => {
  draw({ motivation: motivationOf({ state: QUIET, badges: null }) });
  expect(screen.getAllByTestId('fr-panel')).toHaveLength(6);
  expect(document.body.textContent).not.toMatch(/LEVEL|BADGE EARNED/);
});

// §10.4a: raised above the design's own floors, because it is the affordance criterion 23 rests
// on. The panel is the whole hit target.
test('SHOW WORKING opens the evidence, flips to CLOSE and closes again', () => {
  draw();
  const panel = required(screen.getAllByTestId('fr-panel')[0], 'first panel');
  expect(panel.getAttribute('aria-expanded')).toBe('false');
  expect(within(panel).getByText(copy.SHOW_WORKING_LABEL)).toBeTruthy();
  fireEvent.click(panel);
  expect(panel.getAttribute('aria-expanded')).toBe('true');
  expect(within(panel).getByText(copy.CLOSE_LABEL)).toBeTruthy();
  expect(within(panel).getByText(copy.EVIDENCE_HEADING)).toBeTruthy();
  fireEvent.click(panel);
  expect(panel.getAttribute('aria-expanded')).toBe('false');
});

// §10.4a: the panel is keyboard-reachable and Enter toggles it. What makes Enter work is that
// the panel is a real submit-free <button> in the tab order — jsdom does not implement the
// native key-to-click mapping, so asserting a synthetic keydown here would prove nothing about
// the product. The tag, the type and the focus are the parts a wrong implementation loses.
test('the panel is a real focusable button, which is what makes Enter toggle it', () => {
  draw();
  const panel = required(screen.getAllByTestId('fr-panel')[0], 'first panel');
  expect(panel.tagName).toBe('BUTTON');
  expect(panel.getAttribute('type')).toBe('button');
  expect(panel.getAttribute('tabindex')).toBeNull();
  (panel as HTMLButtonElement).focus();
  expect(document.activeElement).toBe(panel);
});

test('only one panel is open at a time', () => {
  draw();
  const panels = screen.getAllByTestId('fr-panel');
  fireEvent.click(required(panels[0], 'first panel'));
  fireEvent.click(required(panels[3], 'fourth panel'));
  expect(panels[0]?.getAttribute('aria-expanded')).toBe('false');
  expect(panels[3]?.getAttribute('aria-expanded')).toBe('true');
});

// §10.4a: OLDEST STILL ALIVE's evidence stops at the language. The last two fields were GitHub
// visibility and the completion score, both out of phase 1, and `0/10 complete` on an unscored
// repository is "unknown as zero" verbatim.
test('the oldest panel evidence stops at the language', () => {
  draw();
  const panel = required(screen.getAllByTestId('fr-panel')[4], 'fifth panel');
  fireEvent.click(panel);
  expect(within(panel).getByTestId('fr-evidence-body').textContent).toBe(
    'ledger  ·  born 2014  ·  Rust',
  );
  expect(document.body.textContent).not.toMatch(/complete|private|public|Still builds/);
});

// Never render unknown as zero. A figure the core could not compute says so.
test('an uncomputed figure is named and never shown as a number', () => {
  draw({ reveal: { ...reveal(partial), bestYear: { value: null, basis: partial } } });
  const panel = required(screen.getAllByTestId('fr-panel')[3], 'fourth panel');
  expect(within(panel).getByText(copy.UNCOMPUTED_NOTE)).toBeTruthy();
  expect(within(panel).queryByText('0')).toBeNull();
  // Its coverage row is still there: an uncomputed figure is still a figure over a basis.
  expect(within(panel).getByTestId('fr-panel-coverage')).toBeTruthy();
});

// §10.4a: panel 1 takes --sig. The gold top edge belongs to BADGE EARNED (§43.7), drawn through
// `var(--tier-gold)` and so never as `#e8c268` in the markup: it is keyed by its class. With no
// badge drawn — the case drawn here — no panel wears it.
test('with no badge drawn only the first panel wears the accent edge', () => {
  draw({ motivation: motivationOf({ state: shownAt(6), badges: null }) });
  const panels = screen.getAllByTestId('fr-panel');
  const edged = panels.filter((p) => p.className.includes('--sig'));
  expect(edged).toHaveLength(1);
  expect(edged[0]?.textContent).toContain('SPAN');
  expect(panels.filter((p) => p.className.includes('cdt-fr-panel--gold'))).toHaveLength(0);
  expect(document.body.innerHTML).not.toContain('#e8c268');
});

test('GO ON leaves for the turn, and the footer stays as ornament', () => {
  const { onGoOn } = draw();
  expect(screen.getByText(copy.EVIDENCE_FOOTER)).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: copy.GO_ON_LABEL }));
  expect(onGoOn).toHaveBeenCalledTimes(1);
});

// §10.4a: the reveal is where "no fabricated number anywhere" is either kept or lost.
test('no percentage appears on the reveal', () => {
  draw({ reveal: reveal(partial) });
  expect(document.body.textContent).not.toMatch(/%/);
});

const panelNamed = (label: string): HTMLElement =>
  required(
    screen.getAllByTestId('fr-panel').find((p) => p.textContent.startsWith(label)),
    `${label} panel`,
  );

test('AC-P4-43-12-renderer the reveal records the level it displayed and lands the badge once', async () => {
  const acknowledge = vi.fn((): Promise<void> => Promise.resolve());
  const markLanded = vi.fn((): Promise<void> => Promise.resolve());
  const motivation = (state: MotivationState): RevealMotivation =>
    motivationOf({ state, badges: due, acknowledge, markLanded });

  // Displayed *not computed*: the write carries null, so the shelf still owes an arrival.
  const view = draw({ motivation: motivation(shownAt(null)) });
  expect(screen.getAllByTestId('fr-panel')).toHaveLength(8);
  expect(within(panelNamed(LEVEL_LABEL)).getByText(copy.UNCOMPUTED_NOTE)).toBeTruthy();
  expect(acknowledge.mock.calls).toEqual([['revealLevelShown', null]]);

  // Refreshed in place from motivation/changed: the value displayed is sent.
  view.rerender({ motivation: motivation(shownAt(4, 'partial')) });
  expect(within(panelNamed(LEVEL_LABEL)).getByTestId('fr-panel-value').textContent).toBe('4');
  view.rerender({ motivation: motivation(shownAt(4, 'partial')) });
  view.rerender({ motivation: motivation(shownAt(6)) });
  expect(acknowledge.mock.calls).toEqual([
    ['revealLevelShown', null],
    ['revealLevelShown', 4],
    ['revealLevelShown', 6],
  ]);

  // BADGE EARNED: the eighth panel, gold, displayed in an active window — landed once.
  const badge = panelNamed(BADGE_EARNED_LABEL);
  expect(badge.className).toContain('cdt-fr-panel--gold');
  expect(within(badge).queryByTestId('fr-panel-coverage')).toBeNull();
  expect(markLanded.mock.calls).toEqual([['polyglot']]);
  await act(async () => {
    await Promise.resolve();
  });
  expect(markLanded).toHaveBeenCalledTimes(1);
  cleanup();

  // A hidden window, or one without focus, sends nothing — until it comes to the foreground.
  acknowledge.mockClear();
  markLanded.mockClear();
  visibility = 'hidden';
  draw({ motivation: motivation(shownAt(6)) });
  vi.spyOn(document, 'hasFocus').mockReturnValue(false);
  act(() => {
    window.dispatchEvent(new Event('blur'));
  });
  expect(acknowledge).not.toHaveBeenCalled();
  expect(markLanded).not.toHaveBeenCalled();
  visibility = 'visible';
  vi.spyOn(document, 'hasFocus').mockReturnValue(true);
  act(() => {
    document.dispatchEvent(new Event('visibilitychange'));
  });
  expect(acknowledge.mock.calls).toEqual([['revealLevelShown', 6]]);
  expect(markLanded.mock.calls).toEqual([['polyglot']]);
  cleanup();

  // Under quiet neither panel is drawn and nothing is written.
  acknowledge.mockClear();
  markLanded.mockClear();
  draw({ motivation: motivation(QUIET) });
  expect(screen.getAllByTestId('fr-panel')).toHaveLength(6);
  expect(acknowledge).not.toHaveBeenCalled();
  expect(markLanded).not.toHaveBeenCalled();
});

// §43.7: a BADGE EARNED drawn *not computed* that resolves to none is removed in place — the
// other seven keep their elements, so nothing re-cascades.
test('a not-computed badge panel that resolves to none is removed in place', () => {
  const unread: BadgesListed = {
    awards: [],
    due: null,
    basis: { ...full, historyComplete: false },
  };
  const view = draw({ motivation: motivationOf({ state: shownAt(6), badges: unread }) });
  const before = screen.getAllByTestId('fr-panel');
  expect(before).toHaveLength(8);
  expect(within(panelNamed(BADGE_EARNED_LABEL)).getByText(copy.UNCOMPUTED_NOTE)).toBeTruthy();
  view.rerender({
    motivation: motivationOf({ state: shownAt(6), badges: { ...unread, basis: full } }),
  });
  const after = screen.getAllByTestId('fr-panel');
  expect(after).toHaveLength(7);
  after.forEach((panel, i) => {
    expect(panel).toBe(before[i]);
  });
});

import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import { RevealScreen } from './RevealScreen';
import * as copy from './copy';
import type { RevealDeps } from './revealModel';
import type { ProjectId, Reveal, RevealBasis } from '../../generated/protocol';
import { required } from '../../shared/required';

afterEach(cleanup);

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

function draw(over: Partial<Parameters<typeof RevealScreen>[0]> = {}): {
  onGoOn: ReturnType<typeof vi.fn>;
} {
  const onGoOn = vi.fn();
  render(<RevealScreen reveal={reveal()} deps={deps} tier="full" onGoOn={onGoOn} {...over} />);
  return { onGoOn };
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
// that reads as two panels that failed to load.
test('there are exactly six panels and neither phase-4 panel is drawn', () => {
  draw();
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

// §10.4a: panel 1 takes --sig; the gold top edge belongs to BADGE EARNED and is dead.
test('only the first panel wears the accent edge', () => {
  draw();
  const edged = screen.getAllByTestId('fr-panel').filter((p) => p.className.includes('--sig'));
  expect(edged).toHaveLength(1);
  expect(edged[0]?.textContent).toContain('SPAN');
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

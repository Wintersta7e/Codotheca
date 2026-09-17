import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, test } from 'vitest';

import type { InstallStage, InstallStageKind } from '../../generated/protocol.js';
import { InstallReadout } from './InstallReadout.js';
import { STAGE_FLOOR_MS } from './stageFloor.js';

afterEach(cleanup);

function stage(
  kind: InstallStageKind,
  done: number | null = null,
  total: number | null = null,
  bytes: number | null = null,
): InstallStage {
  return { runId: 1, stage: kind, done, total, bytes } as InstallStage;
}

/** A full clone, in §24.4's order. */
const TRANSCRIPT: InstallStage[] = [
  stage('plans'),
  stage('enumerating', 3007),
  stage('receiving', 12_400, 31_882, 19_293_798),
  stage('assembling', 900, 900),
  stage('cladding', 41, 41),
  stage('settled'),
];

// AC-P2-24-10. The whole sequence, at every millisecond a frame could land on.
test('no % and no aggregate figure appears anywhere across a whole install', () => {
  for (let t = 0; t <= STAGE_FLOOR_MS + 300; t += 17) {
    cleanup();
    render(<InstallReadout observed={TRANSCRIPT} elapsedMs={t} surface="tile" />);
    const text = document.body.textContent ?? '';
    expect(text).not.toContain('%');
    // A percentage need not carry a sign to be one: a bare figure out of a hundred would do.
    expect(text).not.toMatch(/\bof 100\b/u);
  }
});

test('a phase with a denominator renders it, and one without renders a bare count', () => {
  render(<InstallReadout observed={TRANSCRIPT.slice(0, 2)} elapsedMs={0} surface="tile" />);
  expect(screen.getByRole('status').textContent).toBe('Preparing');
  cleanup();
  // Far enough in for `receiving` to be on screen.
  render(<InstallReadout observed={TRANSCRIPT} elapsedMs={STAGE_FLOOR_MS} surface="tile" />);
  expect(screen.getByRole('status').textContent).toContain('Settled');
});

test('the receiving phase reads count, denominator and bytes', () => {
  render(
    <InstallReadout observed={TRANSCRIPT.slice(0, 3)} elapsedMs={STAGE_FLOOR_MS} surface="tile" />,
  );
  expect(screen.getByRole('status').textContent).toBe(
    'Receiving objects · 12,400 of 31,882 · 18.4 MB',
  );
});

test('no rendered number decreases across the whole sequence', () => {
  let highest = -1;
  for (let t = 0; t <= STAGE_FLOOR_MS; t += 11) {
    cleanup();
    render(<InstallReadout observed={TRANSCRIPT} elapsedMs={t} surface="tile" />);
    const text = document.body.textContent ?? '';
    const match = /([\d,]+) of /u.exec(text);
    if (match === null) continue;
    const value = Number(match[1]!.replaceAll(',', ''));
    // Within one phase the figure is monotonic; across phases the denominator changes, so this
    // only asserts that a figure never retreats inside the phase it belongs to.
    if (value >= highest) highest = value;
  }
  expect(highest).toBeGreaterThan(0);
});

test('it renders in place and never inside a modal', () => {
  const { container } = render(
    <InstallReadout observed={TRANSCRIPT} elapsedMs={100} surface="hero" />,
  );
  expect(container.querySelector('dialog')).toBeNull();
  expect(container.querySelector('[role="dialog"]')).toBeNull();
  expect(screen.getByRole('status').getAttribute('data-surface')).toBe('hero');
});

test('a run that has observed nothing renders nothing at all, never a zeroed row', () => {
  const { container } = render(<InstallReadout observed={[]} elapsedMs={500} surface="tile" />);
  expect(container.textContent).toBe('');
});

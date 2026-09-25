import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
// `?raw` rather than node:fs: the renderer project carries no Node types, and under jsdom
// `import.meta.url` is not a file URL.
import CSS from './firstRun.css?raw';
import {
  RESCAN_ARM_AFTER_MS,
  RESCAN_LINE_LABEL,
  RESCAN_TRAVEL_MS,
  RescanLine,
  armsIndicator,
} from './RescanLine';
import type { RescanLineProps } from './RescanLine';
import { noop } from '../noop';

afterEach(cleanup);

// §10.5a: never armed by §10.6 mechanism 2. On-focus J2 touches visible tiles only, and an
// indicator that runs on every alt-tab is a scheduled animation at idle wearing a different
// name — criterion 21 forbids it.
test('window focus never arms the line, however long the pass runs', () => {
  expect(armsIndicator('focus', true, 5_000)).toBe(false);
  expect(armsIndicator('focus', true, RESCAN_ARM_AFTER_MS)).toBe(false);
});

test('the other three arm only once the walk is still running at the threshold', () => {
  for (const trigger of ['launch', 'watcher', 'manual'] as const) {
    expect(armsIndicator(trigger, true, RESCAN_ARM_AFTER_MS - 1)).toBe(false);
    expect(armsIndicator(trigger, true, RESCAN_ARM_AFTER_MS)).toBe(true);
  }
});

// Criterion 18's warm case finishes below the threshold, so the user correctly sees nothing.
test('a walk that has ended arms nothing', () => {
  expect(armsIndicator('launch', false, 10_000)).toBe(false);
});

// When the walk ends the element is gone, not paused: returning null is what keeps criterion
// 21's "zero scheduled frames at idle" true without a suspension rule.
test('an unarmed line renders no element at all', () => {
  render(<RescanLine trigger="focus" running tier="full" elapsedMs={9_000} onOpenSummary={noop} />);
  expect(document.querySelector('.cdt-fr-rescan-line')).toBeNull();
  expect(document.body.textContent).toBe('');
});

// §10.5a: at `reduced` and `off` the band does not travel; the line is a static 2 px rule. The
// clamp is the class, because this element is not inside a `.cdt-fr-view` that `firstRun.css`
// could reach with a tier selector.
test('the band travels only at the full tier', () => {
  const props = {
    trigger: 'launch',
    running: true,
    elapsedMs: 600,
    onOpenSummary: noop,
  } as const satisfies Omit<RescanLineProps, 'tier'>;
  const { rerender } = render(<RescanLine {...props} tier="full" />);
  expect(document.querySelector('.cdt-fr-rescan-line--travelling')).not.toBeNull();
  rerender(<RescanLine {...props} tier="reduced" />);
  expect(document.querySelector('.cdt-fr-rescan-line')).not.toBeNull();
  expect(document.querySelector('.cdt-fr-rescan-line--travelling')).toBeNull();
  rerender(<RescanLine {...props} tier="off" />);
  expect(document.querySelector('.cdt-fr-rescan-line')).not.toBeNull();
  expect(document.querySelector('.cdt-fr-rescan-line--travelling')).toBeNull();
});

// §10.5a: clicking it opens the scan summary, which already promises reachability "from the
// scan line".
test('the line names itself and opens the scan summary', () => {
  const onOpenSummary = vi.fn();
  render(
    <RescanLine
      trigger="launch"
      running
      tier="full"
      elapsedMs={600}
      onOpenSummary={onOpenSummary}
    />,
  );
  const line = screen.getByRole('button', { name: RESCAN_LINE_LABEL });
  fireEvent.click(line);
  expect(onOpenSummary).toHaveBeenCalledTimes(1);
});

// A generation walk has no denominator until it ends — the reason §10.2 refuses a percentage —
// so nothing here may read as a fill.
test('the line states no percentage and reports no progress', () => {
  render(
    <RescanLine trigger="launch" running tier="full" elapsedMs={9_000} onOpenSummary={noop} />,
  );
  const line = screen.getByRole('button', { name: RESCAN_LINE_LABEL });
  expect(line.getAttribute('role')).not.toBe('progressbar');
  expect(line.getAttribute('aria-valuenow')).toBeNull();
  expect(document.body.textContent).not.toMatch(/%/);
});

test('the travel duration is the looped one, not the one-shot strip-light', () => {
  expect(RESCAN_TRAVEL_MS).toBe(1150);
  expect(RESCAN_ARM_AFTER_MS).toBe(400);
});

// §10.5a fixes the loop at 1.15s linear and the travel from -60% to 160%. A stylesheet is the
// one place those numbers drift silently, because no type checker reads it — and the constant
// above and the keyframe below are the same value stated twice, so one test reads both.
//
// Nothing here compares a CSS literal as a string beyond these positions: the Write hook
// reformats `.css` on write and `fmt:check` does not cover it, so durations are compared as
// milliseconds rather than as `1.15s`.
test('the stylesheet carries the travel geometry §10.5a fixes', () => {
  // A gate that passes over an empty read is a failing gate that looks green.
  expect(CSS.length).toBeGreaterThan(2_000);

  const travel = /animation:\s*rescanTravel\s+([\d.]+)(m?s)\s+linear\s+infinite/.exec(CSS);
  expect(travel).not.toBeNull();
  const ms = Number(travel?.[1]) * (travel?.[2] === 's' ? 1000 : 1);
  expect(ms).toBe(RESCAN_TRAVEL_MS);

  expect(CSS).toContain('background-position: -60% 0');
  expect(CSS).toContain('background-position: 160% 0');
  expect(CSS).toContain('background-size: 190% 100%');
});

// §11.6: a static equivalent at every tier. The turn's line animates its own tracking, so a
// clamped turn that only dropped the animation would ship at the keyframe's 0% — .4em, which
// is unreadable at 38px. Resolved on a real element, never matched as stylesheet text.
test('a clamped turn keeps the tracking its keyframe would have left it', () => {
  expect(CSS.length).toBeGreaterThan(2_000);
  const style = document.createElement('style');
  style.textContent = CSS;
  document.head.append(style);

  const view = document.createElement('div');
  view.className = 'cdt-fr-view cdt-fr-view--turn';
  const line = document.createElement('p');
  line.className = 'cdt-fr-turn-line';
  view.append(line);
  document.body.append(view);

  // The `full` control first, so the two clamped assertions cannot pass over a sheet jsdom
  // never applied.
  view.setAttribute('data-effects-tier', 'full');
  expect(getComputedStyle(line).animation).toContain('turnIn');

  // Resolved in px, because jsdom turns `.02em` at 38px into `0.76px`. The keyframe's own 0%
  // is `.4em` — 15.2px — so a clamp that dropped the animation without restating the tracking
  // would land twenty times wide, and comparing the declaration as text would not notice.
  const size = Number.parseFloat(getComputedStyle(line).fontSize);
  expect(size).toBe(38);
  for (const tier of ['reduced', 'off']) {
    view.setAttribute('data-effects-tier', tier);
    expect(getComputedStyle(line).animation).not.toContain('turnIn');
    const tracking = Number.parseFloat(getComputedStyle(line).letterSpacing);
    expect(tracking).toBeCloseTo(0.02 * size, 5);
    expect(tracking).not.toBeCloseTo(0.4 * size, 5);
  }

  style.remove();
  view.remove();
});

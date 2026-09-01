import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import {
  RESCAN_ARM_AFTER_MS,
  RESCAN_LINE_LABEL,
  RESCAN_TRAVEL_MS,
  RescanLine,
  armsIndicator,
} from './RescanLine';
import type { RescanLineProps } from './RescanLine';

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
  render(
    <RescanLine trigger="focus" running tier="full" elapsedMs={9_000} onOpenSummary={() => {}} />,
  );
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
    onOpenSummary: () => {},
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
    <RescanLine trigger="launch" running tier="full" elapsedMs={9_000} onOpenSummary={() => {}} />,
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

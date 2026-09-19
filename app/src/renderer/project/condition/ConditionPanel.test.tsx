import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { ConditionSignal } from '../../../generated/protocol';
import { conditionDot, DOT_SIZE_PX } from '../../derive/condition';
import { ConditionPanel } from './ConditionPanel';
import { needleAngle } from './dial';

afterEach(cleanup);

const ALL: readonly ConditionSignal[] = [
  'live',
  'idle',
  'dormant',
  'neglected',
  'abandoned',
  'offline',
  'empty',
];

function draw(
  signal: ConditionSignal | null,
  material: ConditionSignal | null,
  over: { isReference?: boolean; isArchived?: boolean } = {},
): HTMLElement {
  const { container } = render(
    <ConditionPanel
      signal={signal}
      material={material}
      isReference={over.isReference ?? false}
      isArchived={over.isArchived ?? false}
    />,
  );
  return container;
}

function needle(container: HTMLElement, which: 'signal' | 'material'): HTMLElement | null {
  return container.querySelector<HTMLElement>(`.cp-needle[data-needle="${which}"]`);
}

/**
 * **`AC-P3-33-11`** — the two clocks, side by side.
 *
 * The fills are read back off `conditionDot` rather than compared to hexes written here. **A
 * second copy of §5.4a's table is the defect criterion 58 exists to catch**, so this file states
 * no colour of its own.
 */
describe('the CONDITION panel', () => {
  it('ac_p3_33_11 draws both needles from §5.4as one owner, at their ladder angles', () => {
    const container = draw('live', 'neglected');
    const outer = needle(container, 'signal');
    const inner = needle(container, 'material');
    expect(outer).not.toBeNull();
    expect(inner).not.toBeNull();
    expect(outer?.style.transform).toBe(`rotate(${String(needleAngle('live'))}deg)`);
    expect(inner?.style.transform).toBe(`rotate(${String(needleAngle('neglected'))}deg)`);
  });

  it('emits no fill or ring §5.4as table does not contain, over every variant', () => {
    let checked = 0;
    for (const signal of ALL) {
      const container = draw(signal, signal);
      const expected = conditionDot({ signal, isReference: false, isArchived: false });
      const outer = needle(container, 'signal');
      expect(outer?.style.getPropertyValue('--cdt-needle-fill')).toBe(
        expected?.fill ?? 'transparent',
      );
      expect(outer?.style.getPropertyValue('--cdt-needle-ring')).toBe(expected?.ring ?? 'none');
      checked += 1;
      cleanup();
    }
    expect(checked).toBe(ALL.length);
  });

  it('honours the two overrides §5.4a evaluates before the band', () => {
    const reference = draw('live', 'live', { isReference: true });
    const expected = conditionDot({ signal: 'live', isReference: true, isArchived: false });
    expect(needle(reference, 'signal')?.style.getPropertyValue('--cdt-needle-ring')).toBe(
      expected?.ring ?? 'none',
    );
  });

  it('draws NO inner needle and NO divergence line when the material clock is NULL', () => {
    // Assert the emptiness of the node, not the absence of a digit: a needle at angle 0 with no
    // fill would pass a digits-only check.
    const container = draw('live', null);
    expect(needle(container, 'signal')).not.toBeNull();
    expect(needle(container, 'material')).toBeNull();
    expect(container.querySelectorAll('.cp-needle').length).toBe(1);
    expect(container.querySelector('[data-testid="cp-condition-divergence"]')).toBeNull();
  });

  it('draws the needles and no divergence line when either value is off the ladder', () => {
    for (const pair of [
      ['offline', 'live'],
      ['live', 'empty'],
      ['empty', 'offline'],
    ] as const) {
      const container = draw(pair[0], pair[1]);
      expect(container.querySelectorAll('.cp-needle').length).toBe(2);
      expect(container.querySelector('[data-testid="cp-condition-divergence"]')).toBeNull();
      cleanup();
    }
  });

  it('reads the three divergences and nothing else', () => {
    const read = (s: ConditionSignal, m: ConditionSignal): string | undefined =>
      draw(s, m).querySelector('[data-testid="cp-condition-divergence"]')?.textContent ?? undefined;
    expect(read('idle', 'idle')).toBe('IN STEP');
    cleanup();
    expect(read('live', 'abandoned')).toBe('LIT BUT DUSTY');
    cleanup();
    expect(read('abandoned', 'live')).toBe('CLEAN BUT DARK');
  });

  it('draws the condition line at §5.4as deferred 6px, and it is a surface of its own', () => {
    const container = draw('idle', 'idle');
    const dot = container.querySelector<HTMLElement>('.cp-condition-dot');
    expect(dot).not.toBeNull();
    expect(dot?.getAttribute('data-dot-surface')).toBe('conditionLine');
    expect(dot?.style.width).toBe(`${String(DOT_SIZE_PX.conditionLine)}px`);
    expect(dot?.style.height).toBe(`${String(DOT_SIZE_PX.conditionLine)}px`);
    expect(DOT_SIZE_PX.conditionLine).toBe(6);
    // The identity line's dot is a different row of the same table and keeps its own size.
    expect(DOT_SIZE_PX.projectPage).toBe(9);
  });

  it('names the mark through the one accessible-name owner and carries no band vocabulary', () => {
    const container = draw('dormant', 'dormant');
    expect(container.querySelector('.cdt-visually-hidden')?.textContent).toBe('Condition: dormant');
    // Criterion 58, unchanged: the design's band vocabulary is forbidden everywhere.
    const html = container.innerHTML.toLowerCase();
    expect(html).not.toContain('warm');
    expect(html).not.toContain('cooling');
  });

  it('draws no outer needle when nothing has ever been indexed', () => {
    // `condition_signal IS NULL` draws no dot at all on every surface (criterion 58); the
    // NOT INDEXED badge carries the state.
    const container = draw(null, null);
    expect(container.querySelectorAll('.cp-needle').length).toBe(0);
    expect(container.querySelector('[data-testid="cp-condition-divergence"]')).toBeNull();
  });
});

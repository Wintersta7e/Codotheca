import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { AttentionChip } from './AttentionChip.js';
import { ERA_CHEVRON, EraHeader } from './EraHeader.js';
import type { ShelfSection } from './page.js';
import shelfCss from './shelf.css?raw';

/**
 * §8.1's era header and §8.0b's attention chip, resolved on the mounted components under the real
 * `shelf.css` rather than read out of the stylesheet text.
 *
 * jsdom never matches `:hover`, so hover is forced: every `:hover` in the stylesheet becomes
 * `[data-forced-hover]` — one pseudo-class for one attribute selector, the same specificity — and
 * the attribute is set on the element. The cascade, the tier clamps included, then decides the
 * hovered style exactly as it decides the resting one. jsdom does not substitute `var()`, so a
 * token is asserted as the reference the element resolves to; it does resolve `em` to px, so
 * tracking is compared as the ratio of the two.
 */
const FORCED = '[data-forced-hover]';

afterEach(() => {
  cleanup();
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
});

function mountShelfCss(tier: 'full' | 'reduced' | 'off' = 'full'): void {
  expect(shelfCss.length, 'shelf.css imported as an empty string').toBeGreaterThan(0);
  const forced = shelfCss.replaceAll(':hover', FORCED);
  expect(forced, 'shelf.css declares no hover to force').not.toBe(shelfCss);
  const style = document.createElement('style');
  style.textContent = forced;
  document.head.append(style);
  document.documentElement.setAttribute('data-effects-tier', tier);
}

function styleOf(selector: string): CSSStyleDeclaration {
  const node = document.querySelector(selector);
  if (node === null) throw new Error(`nothing mounted ${selector}`);
  return getComputedStyle(node);
}

function hover(selector: string): void {
  const node = document.querySelector(selector);
  if (node === null) throw new Error(`nothing mounted ${selector}`);
  node.setAttribute('data-forced-hover', '');
}

const px = (value: string): number => {
  const match = /^(-?\d*\.?\d+)px$/.exec(value.trim());
  if (match === null) throw new Error(`not a px length: ${value}`);
  return Number(match[1]);
};
/** Tracking in em, from the resolved px tracking over the resolved px size, to two places. */
const tracking = (style: CSSStyleDeclaration): number =>
  Math.round((px(style.letterSpacing) / px(style.fontSize)) * 100) / 100;
const translateY = (transform: string): number => {
  const match = /^translateY\((-?\d*\.?\d+)px\)$/.exec(transform.trim());
  if (match === null) throw new Error(`not a translateY: ${transform}`);
  return Number(match[1]);
};

const section: ShelfSection = {
  id: 'era:2019',
  order: 17,
  year: 2019,
  cutAgainstYear: 2026,
  label: '2019',
  agg: {
    count: 212,
    trackedBytes: 44 * 1024 ** 3,
    indexedCount: 198,
    unpushed: 8,
    uncommitted: 3,
    interrupted: 3,
    unchecked: 0,
  },
  rows: [],
};

const drawHeader = (collapsed: boolean): void => {
  render(
    <EraHeader section={section} collapsed={collapsed} onToggle={vi.fn()} probe={() => false} />,
  );
};

describe('§8.1: the era header in the design’s terse form', () => {
  it('draws ▾ open and ▸ collapsed, as a glyph outside the accessible name', () => {
    drawHeader(false);
    const open = document.querySelector('.cdt-era-chevron .cdt-era-chev');
    expect(open?.textContent).toBe(ERA_CHEVRON.expanded);
    expect(open?.getAttribute('aria-hidden')).toBe('true');
    expect(document.querySelector('.cdt-era-chevron')?.getAttribute('aria-label')).not.toMatch(
      /[▾▸]/,
    );
    cleanup();
    drawHeader(true);
    expect(document.querySelector('.cdt-era-chev')?.textContent).toBe(ERA_CHEVRON.collapsed);
    expect(ERA_CHEVRON).toEqual({ expanded: '▾', collapsed: '▸' });
  });

  it('puts the glyph first, then the name, the summary and the flags', () => {
    drawHeader(false);
    const parts = [...(document.querySelector('.cdt-era-chevron')?.children ?? [])].map(
      (node) => node.className,
    );
    expect(parts).toEqual(['cdt-era-chev', 'cdt-era-label', 'cdt-era-summary', 'cdt-era-flags']);
  });

  it('sets the name at 15px .14em, the summary at 9px .1em in --text-3', () => {
    mountShelfCss();
    drawHeader(false);
    const label = styleOf('.cdt-era-label');
    expect(px(label.fontSize)).toBe(15);
    expect(tracking(label)).toBe(0.14);
    expect(label.fontFamily).toBe('var(--font-display)');
    const summary = styleOf('.cdt-era-summary');
    expect(px(summary.fontSize)).toBe(9);
    expect(tracking(summary)).toBe(0.1);
    expect(summary.color).toBe('var(--text-3)');
  });

  it('pushes the flags to the right edge in --sig, never shrinking them', () => {
    mountShelfCss();
    drawHeader(false);
    const button = styleOf('.cdt-era-chevron');
    expect(button.flexGrow).toBe('1');
    const flags = styleOf('.cdt-era-flags');
    expect(flags.color).toBe('var(--sig)');
    expect(flags.marginLeft).toBe('auto');
    expect(flags.flexShrink).toBe('0');
    expect(flags.whiteSpace).toBe('nowrap');
  });

  it('lights the glyph in --sig under the pointer', () => {
    mountShelfCss();
    drawHeader(false);
    expect(styleOf('.cdt-era-chev').color).toBe('var(--text-3)');
    hover('.cdt-era-chevron');
    expect(styleOf('.cdt-era-chev').color).toBe('var(--sig)');
  });
});

const drawChip = (active = false): void => {
  render(
    <AttentionChip
      count={4}
      label="UNPUSHED"
      subLine="WORK ONLY ON THIS DISK"
      accent="var(--sig)"
      active={active}
      onActivate={vi.fn()}
    />,
  );
};

describe('§8.0b: the attention chip is a row with an accent number', () => {
  it('lays out as a centred row: the number, then the label over its sub-line', () => {
    mountShelfCss();
    drawChip();
    const chip = styleOf('.cdt-attention-chip');
    expect(chip.display).toBe('flex');
    expect(chip.flexDirection).not.toBe('column');
    expect(chip.alignItems).toBe('center');
    expect(px(chip.gap)).toBe(12);
    expect([chip.paddingTop, chip.paddingRight, chip.paddingBottom].map(px)).toEqual([10, 15, 11]);
    const button = document.querySelector('.cdt-attention-chip');
    expect(button?.firstElementChild?.className).toBe('cdt-attention-count');
  });

  it('sets the number in the accent at 27px/700, line-height .86', () => {
    mountShelfCss();
    drawChip();
    const count = styleOf('.cdt-attention-count');
    expect(count.color).toBe('var(--cdt-chip-accent)');
    expect(px(count.fontSize)).toBe(27);
    expect(count.fontWeight).toBe('700');
    expect(Number(count.lineHeight)).toBe(0.86);
    expect(count.fontFamily).toBe('var(--font-display)');
  });

  it('sets the label in the display face, 13px/700 .11em, and the sub-line mono 7.5px .12em', () => {
    mountShelfCss();
    drawChip();
    const label = styleOf('.cdt-attention-label');
    expect(label.fontFamily).toBe('var(--font-display)');
    expect(px(label.fontSize)).toBe(13);
    expect(label.fontWeight).toBe('700');
    expect(tracking(label)).toBe(0.11);
    expect(label.color).toBe('var(--text-1)');
    const sub = styleOf('.cdt-attention-sub');
    expect(sub.fontFamily).toBe('var(--font-mono)');
    expect(px(sub.fontSize)).toBe(7.5);
    expect(tracking(sub)).toBe(0.12);
    expect(sub.color).toBe('var(--text-3)');
  });

  it('lifts 1px on hover, takes --surface-4 and lifts the sub-line to --text-2', () => {
    mountShelfCss();
    drawChip();
    expect(styleOf('.cdt-attention-chip').transform).toBe('none');
    hover('.cdt-attention-chip');
    const chip = styleOf('.cdt-attention-chip');
    expect(translateY(chip.transform)).toBe(-1);
    expect(chip.backgroundColor).toBe('var(--surface-4)');
    expect(styleOf('.cdt-attention-sub').color).toBe('var(--text-2)');
  });

  it('keeps the hover ground but drops the lift below full', () => {
    for (const tier of ['reduced', 'off'] as const) {
      mountShelfCss(tier);
      drawChip();
      hover('.cdt-attention-chip');
      const chip = styleOf('.cdt-attention-chip');
      expect(chip.transform, `lift at ${tier}`).toBe('none');
      expect(chip.backgroundColor, `ground at ${tier}`).toBe('var(--surface-4)');
      cleanup();
      document.head.querySelectorAll('style').forEach((node) => {
        node.remove();
      });
    }
  });

  // §8.0b: "On --surface-4, hover or active, the sub-line renders --text-2"; §8.8 gives every chip
  // the active ground, the accent outline and a --text-0 label. No pointer is involved: the
  // pressed state alone must carry it, and it must not lift.
  it('takes --surface-4 and lifts the sub-line to --text-2 while pressed, without the pointer', () => {
    mountShelfCss();
    drawChip(true);
    expect(document.querySelector('.cdt-attention-chip')?.getAttribute('aria-pressed')).toBe(
      'true',
    );
    const chip = styleOf('.cdt-attention-chip');
    expect(chip.backgroundColor).toBe('var(--surface-4)');
    expect(chip.transform).toBe('none');
    expect(chip.boxShadow.match(/var\(--cdt-chip-accent\)/g)).toHaveLength(2);
    expect(chip.boxShadow).not.toContain('var(--line-2)');
    expect(styleOf('.cdt-attention-label').color).toBe('var(--text-0)');
    expect(styleOf('.cdt-attention-sub').color).toBe('var(--text-2)');
  });

  it('keeps the resting ground and sub-line while not pressed', () => {
    mountShelfCss();
    drawChip(false);
    expect(document.querySelector('.cdt-attention-chip')?.getAttribute('aria-pressed')).toBe(
      'false',
    );
    expect(styleOf('.cdt-attention-chip').backgroundColor).toBe('var(--surface-2)');
    expect(styleOf('.cdt-attention-label').color).toBe('var(--text-1)');
    expect(styleOf('.cdt-attention-sub').color).toBe('var(--text-3)');
  });
});

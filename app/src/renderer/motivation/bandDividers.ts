import { type RefObject, useLayoutEffect } from 'react';

/**
 * §43.2's band wraps (`flex-wrap: wrap`), and a divider separates two groups only while both
 * share its row. One the wrap leaves opening or closing a row separates nothing, so it is
 * stranded: one that opens a row is taken out of the flow, and one that closes a row keeps its box
 * and stops painting — taking that one out would free the room the next group lacked, pull the
 * group back up and strand the divider again, a loop a kept box cannot enter.
 */
export type Strand = 'start' | 'end' | null;

/** One direct child of the band, as laid out: its vertical extent, and whether it is a divider. */
export interface BandItem {
  readonly divider: boolean;
  readonly top: number;
  readonly bottom: number;
}

const DIVIDER_CLASS = 'cdt-band-divider';
const STRANDED = 'data-stranded';

/** An item with no height — the spacer, anything not laid out — is nobody's neighbour. */
const painted = (item: BandItem): boolean => item.bottom > item.top;

const sameRow = (a: BandItem, b: BandItem): boolean => a.top < b.bottom && b.top < a.bottom;

/** For each item in DOM order: whether it is a painted divider that opens or closes its row. */
export function strandedDividers(items: readonly BandItem[]): Strand[] {
  return items.map((item, index) => {
    if (!item.divider || !painted(item)) return null;
    const before = items
      .slice(0, index)
      .reverse()
      .find((other) => !other.divider && painted(other));
    const after = items.slice(index + 1).find((other) => !other.divider && painted(other));
    if (before === undefined || !sameRow(before, item)) return 'start';
    if (after === undefined || !sameRow(item, after)) return 'end';
    return null;
  });
}

/**
 * Re-reads the band's rows after every render and on every change of the band's size, and marks
 * each stranded divider with `data-stranded`, which `band.css` acts on. Each pass first clears the
 * marks, so a divider is judged where it would sit; reading and writing in one pass, before paint,
 * shows no frame with a stranded divider.
 */
export function useSettledDividers(band: RefObject<HTMLElement | null>): void {
  useLayoutEffect(() => {
    const target = band.current;
    if (target === null) return undefined;
    const settle = (): void => {
      const children = [...target.children];
      for (const child of children) {
        if (child.classList.contains(DIVIDER_CLASS)) child.removeAttribute(STRANDED);
      }
      const items = children.map((child) => {
        const box = child.getBoundingClientRect();
        return {
          divider: child.classList.contains(DIVIDER_CLASS),
          top: box.top,
          bottom: box.bottom,
        };
      });
      strandedDividers(items).forEach((strand, index) => {
        if (strand !== null) children[index]?.setAttribute(STRANDED, strand);
      });
    };
    settle();
    if (typeof ResizeObserver === 'undefined') return undefined;
    const observer = new ResizeObserver(settle);
    observer.observe(target);
    return () => {
      observer.disconnect();
    };
  });
}

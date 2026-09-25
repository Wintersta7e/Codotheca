import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectId } from '../../generated/protocol.js';
import { conditionDot } from '../derive/condition.js';
import { ReferenceTail, referenceSummary } from './ReferenceTail.js';
import type { ShelfRow } from './row.js';
import { noop } from '../noop.js';

afterEach(cleanup);

const rows = (n: number, over: Record<string, unknown> = {}): ShelfRow[] =>
  Array.from(
    { length: n },
    (_, i) =>
      ({
        id: i + 1,
        name: `ref${String(i + 1)}`,
        isReference: true,
        isArchived: false,
        conditionSignal: null,
        sizeTrackedBytes: null,
        primaryLanguage: null,
        ...over,
      }) as unknown as ShelfRow,
  );

describe('ReferenceTail', () => {
  it('is uncapped — 41 rows render 41 rows, not 40', () => {
    render(<ReferenceTail rows={rows(41)} onOpen={noop} />);
    expect(document.querySelectorAll('.cdt-reference-row')).toHaveLength(41);
  });

  it('names only what phase 1 has', () => {
    expect(referenceSummary(24)).toBe(
      '24 repositories with no commits by you · excluded from your stats',
    );
    for (const word of ['decay', 'health', 'quest']) {
      expect(referenceSummary(24)).not.toContain(word);
    }
  });

  it('renders nothing at all when there is no Reference set', () => {
    const { container } = render(<ReferenceTail rows={[]} onOpen={noop} />);
    expect(container.firstChild).toBeNull();
  });

  it('carries the REFERENCE bar', () => {
    render(<ReferenceTail rows={rows(2)} onOpen={noop} />);
    expect(screen.getByText('REFERENCE')).toBeTruthy();
  });

  it('draws §5.4a’s reference ring, from conditionDot alone, on every row', () => {
    // The plan's own test asserts NO dot here. That contradicts two settled things: `conditionDot`
    // returns the reference ring before it ever reads `condition_signal`
    // (`derive/condition.ts:54-57`), and the design's reference row carries an unfilled 7px ring
    // on every row (`Codotheca v7 Shelf.dc.html:471`). Criterion 58's "draws none when
    // condition_signal IS NULL" is about the LIST row, where `isReference` is false.
    render(<ReferenceTail rows={rows(3)} onOpen={noop} />);
    const dots = [...document.querySelectorAll('.cdt-condition-dot')] as HTMLElement[];
    expect(dots).toHaveLength(3);
    const expected = conditionDot({ signal: null, isReference: true, isArchived: false });
    expect(expected).not.toBeNull();
    expect(dots[0]?.style.getPropertyValue('--cdt-dot-ring')).toBe(expected?.ring);
    // A ring, never a filled disc: an unfilled mark is what says "not scored", not a band colour.
    expect(dots[0]?.style.getPropertyValue('--cdt-dot-fill')).toBe('transparent');
  });

  it('does not announce a ring that names nothing', () => {
    // The mark means "reference", the block is already called REFERENCE, and `conditionDotName`
    // has no name for a NULL signal. An unnamed decorative node is hidden rather than announced.
    render(<ReferenceTail rows={rows(1)} onOpen={noop} />);
    const dot = document.querySelector('.cdt-condition-dot');
    expect(dot?.getAttribute('aria-hidden')).toBe('true');
    expect(dot?.getAttribute('aria-label')).toBeNull();
  });

  it('names the dot when there is a signal to name, and never by its colour', () => {
    render(<ReferenceTail rows={rows(1, { conditionSignal: 'dormant' })} onOpen={noop} />);
    const dot = document.querySelector('.cdt-condition-dot');
    expect(dot?.getAttribute('aria-label')).toBe('Condition: dormant');
    expect(dot?.getAttribute('aria-hidden')).toBeNull();
  });

  it('renders no size figure for an unmeasured inventory, and one for a measured zero', () => {
    render(<ReferenceTail rows={rows(1)} onOpen={noop} />);
    expect(document.querySelector('.cdt-reference-size')?.textContent).toBe('—');
    cleanup();
    render(<ReferenceTail rows={rows(1, { sizeTrackedBytes: 0 })} onOpen={noop} />);
    expect(document.querySelector('.cdt-reference-size')?.textContent).toBe('0 MB');
  });

  it('opens the project it was clicked on', () => {
    const onOpen = vi.fn();
    render(<ReferenceTail rows={rows(3)} onOpen={onOpen} />);
    (document.querySelectorAll('.cdt-reference-row')[2] as HTMLElement).click();
    expect(onOpen).toHaveBeenCalledWith(3 as unknown as ProjectId);
  });

  it('carries no roast and no completion readout — it is not that surface', () => {
    const { container } = render(<ReferenceTail rows={rows(4)} onOpen={noop} />);
    expect(container.querySelector('.cdt-roast')).toBeNull();
    expect(container.textContent).not.toMatch(/EVALUABLE|UNKNOWN|0\s*\/\s*10/);
  });
});

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { EraHeader } from './EraHeader.js';
import type { SectionAggregate } from './eras.js';
import { summaryText, summaryTextTruncated } from './eras.js';
import type { ShelfSection } from './page.js';

afterEach(cleanup);

const agg = (over: Partial<SectionAggregate> = {}): SectionAggregate => ({
  count: 212,
  trackedBytes: 44 * 1024 ** 3,
  indexedCount: 198,
  unpushed: 8,
  uncommitted: 3,
  interrupted: 3,
  unchecked: 0,
  ...over,
});
const section = (over: Partial<ShelfSection> = {}): ShelfSection => ({
  id: 'era:2019',
  order: 17,
  year: 2019,
  cutAgainstYear: 2026,
  label: '2019',
  agg: agg(),
  rows: [],
  ...over,
});

describe('EraHeader', () => {
  it('is a real button carrying aria-expanded, named by its summary (§11.7)', () => {
    render(<EraHeader section={section()} collapsed onToggle={() => {}} probe={() => false} />);
    const button = screen.getByRole('button');
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(button.getAttribute('aria-label')).toContain('212 projects');
  });

  it('flips aria-expanded with the section, so the chevron is not the only signal', () => {
    render(
      <EraHeader section={section()} collapsed={false} onToggle={() => {}} probe={() => false} />,
    );
    expect(screen.getByRole('button').getAttribute('aria-expanded')).toBe('true');
  });

  it('toggles', () => {
    const onToggle = vi.fn();
    render(
      <EraHeader section={section()} collapsed={false} onToggle={onToggle} probe={() => false} />,
    );
    screen.getByRole('button').click();
    expect(onToggle).toHaveBeenCalledOnce();
  });

  it('drops the byte figure with its parenthetical as one unit when it does not fit', () => {
    render(
      <EraHeader section={section()} collapsed={false} onToggle={() => {}} probe={() => true} />,
    );
    const summary = document.querySelector('.cdt-era-summary');
    expect(summary?.textContent).toContain('212 projects');
    expect(summary?.textContent).not.toContain('tracked');
    expect(summary?.textContent).not.toContain('indexed');
    // The truncated form is the one the model owns, not a string this component composed.
    expect(summary?.textContent).toBe(summaryTextTruncated(agg()));
  });

  it('keeps the whole summary when it fits', () => {
    render(
      <EraHeader section={section()} collapsed={false} onToggle={() => {}} probe={() => false} />,
    );
    const summary = document.querySelector('.cdt-era-summary');
    expect(summary?.textContent).toContain('tracked');
    expect(summary?.textContent).toContain('of 198 indexed');
    expect(summary?.textContent).toBe(summaryText(agg()));
  });

  it('measures once and settles, rather than oscillating between the two forms', () => {
    // The probe is called against the full string; once truncated the short one fits by
    // construction, so a probe re-run on the short form would restore the long one forever.
    const probe = vi.fn(() => true);
    render(<EraHeader section={section()} collapsed={false} onToggle={() => {}} probe={probe} />);
    expect(document.querySelector('.cdt-era-summary')?.textContent).toBe(
      summaryTextTruncated(agg()),
    );
    expect(probe.mock.calls.length).toBeLessThan(4);
  });

  it('re-measures at full width when the aggregate changes', () => {
    // A section whose count changes mid-scan must be measured again, and measured against the
    // full string — not against the short one it happens to be showing.
    let fits = false;
    const { rerender } = render(
      <EraHeader section={section()} collapsed={false} onToggle={() => {}} probe={() => !fits} />,
    );
    expect(document.querySelector('.cdt-era-summary')?.textContent).toBe(
      summaryTextTruncated(agg()),
    );
    fits = true;
    const grown = agg({ count: 4 });
    rerender(
      <EraHeader
        section={section({ agg: grown })}
        collapsed={false}
        onToggle={() => {}}
        probe={() => !fits}
      />,
    );
    expect(document.querySelector('.cdt-era-summary')?.textContent).toBe(summaryText(grown));
  });

  it('renders the flag line when every observed count is zero and unchecked is not', () => {
    render(
      <EraHeader
        section={section({
          agg: agg({ unpushed: 0, uncommitted: 0, interrupted: 0, unchecked: 7 }),
        })}
        collapsed={false}
        onToggle={() => {}}
        probe={() => false}
      />,
    );
    expect(document.querySelector('.cdt-era-flags')?.textContent).toContain('7 unchecked');
  });

  it('renders no flag line at all when there is nothing to report', () => {
    render(
      <EraHeader
        section={section({
          agg: agg({ unpushed: 0, uncommitted: 0, interrupted: 0, unchecked: 0 }),
        })}
        collapsed={false}
        onToggle={() => {}}
        probe={() => false}
      />,
    );
    expect(document.querySelector('.cdt-era-flags')).toBeNull();
  });

  it('never truncates the flag line, whatever the summary does', () => {
    render(
      <EraHeader section={section()} collapsed={false} onToggle={() => {}} probe={() => true} />,
    );
    expect(document.querySelector('.cdt-era-flags')?.textContent).toBe(
      '8 unpushed · 3 uncommitted · 3 interrupted',
    );
  });

  it('carries the label the model composed, never a year it derived itself', () => {
    render(
      <EraHeader
        section={section({ id: 'era:tail', label: '2015 AND EARLIER' })}
        collapsed
        onToggle={() => {}}
        probe={() => false}
      />,
    );
    expect(document.querySelector('.cdt-era-label')?.textContent).toBe('2015 AND EARLIER');
  });
});

/**
 * AC-P2-23-5's first half, asserted over the **rendered** header and over its accessible name.
 *
 * The rule lives in `summaryText`, not here: `EraHeader` builds the chevron's accessible name
 * from the un-truncated string, so a rule applied at the render site would leave `0 MB tracked`
 * in an accessible name while removing it from the screen. One owner per value.
 */
describe('§23.4: a total over zero measurements is not a measurement', () => {
  const both = (el: Element | null, button: HTMLElement): string =>
    `${el?.textContent ?? ''}|${button.getAttribute('aria-label') ?? ''}`;

  it('drops the byte aggregate and its parenthetical for a not-cloned section', () => {
    render(
      <EraHeader
        section={section({
          id: 'era:notcloned',
          label: 'NOT CLONED',
          order: 98,
          year: null,
          agg: agg({
            count: 41,
            trackedBytes: 0,
            indexedCount: 0,
            unpushed: 0,
            uncommitted: 0,
            interrupted: 0,
            unchecked: 0,
          }),
        })}
        collapsed={false}
        onToggle={() => {}}
        probe={() => false}
      />,
    );
    const button = screen.getByRole('button');
    const text = both(document.querySelector('.cdt-era-summary'), button);
    expect(document.querySelector('.cdt-era-summary')?.textContent).toBe('41 projects');
    expect(text).not.toContain('0 MB tracked');
    expect(text).not.toContain('tracked');
    expect(text).not.toContain('indexed');
  });

  /**
   * The case that proves the rule is **one rule**: a located section during a live scan, before
   * any inventory has completed, makes the same false claim one surface earlier. The
   * implementation branches on `indexedCount === 0` and never on the section id.
   */
  it('drops it for a located section with no completed inventory either', () => {
    render(
      <EraHeader
        section={section({
          id: 'era:live',
          label: 'LIVE',
          order: 0,
          year: null,
          agg: agg({
            count: 7,
            trackedBytes: 0,
            indexedCount: 0,
            unpushed: 0,
            uncommitted: 0,
            interrupted: 0,
            unchecked: 7,
          }),
        })}
        collapsed={false}
        onToggle={() => {}}
        probe={() => false}
      />,
    );
    const button = screen.getByRole('button');
    const text = both(document.querySelector('.cdt-era-summary'), button);
    expect(document.querySelector('.cdt-era-summary')?.textContent).toBe('7 projects');
    expect(text).not.toContain('0 MB tracked');
    // The flag line is a different rule and still renders: those seven rows have a working copy
    // and nothing has checked it.
    expect(document.querySelector('.cdt-era-flags')?.textContent).toBe('7 unchecked');
  });

  it('keeps the aggregate the moment one row has been measured', () => {
    render(
      <EraHeader
        section={section({ agg: agg({ count: 7, trackedBytes: 1024 ** 2, indexedCount: 1 }) })}
        collapsed={false}
        onToggle={() => {}}
        probe={() => false}
      />,
    );
    expect(document.querySelector('.cdt-era-summary')?.textContent).toBe(
      '7 projects · 1 MB tracked (of 1 indexed)',
    );
  });
});

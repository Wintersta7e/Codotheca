import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ATTENTION_CHIPS, attentionCounts } from './counts.js';
import {
  AttentionChip,
  AttentionRow,
  SPACE_PEEK_HINT,
  type AttentionRowProps,
} from './AttentionRow.js';
import type { QueryContext } from './evaluate.js';
import type { ShelfRow } from './row.js';

afterEach(cleanup);

const ctx = {
  now: 0,
  firstRunCompletedAt: null,
  collectionIdsByName: new Map(),
  pathsAreCaseSensitive: false,
  commitSubjectHits: null,
  capabilities: {
    authoredByUser: false,
    location: false,
    hasReadme: false,
    hasLicense: false,
    hasTests: false,
    hasCi: false,
    hasRemote: false,
    hasSubmodules: false,
  },
} as unknown as QueryContext;
const counts = {
  matched: 12,
  total: 180,
  reference: 24,
  classified: 141,
  classificationKnown: false,
};
const rows = [{ id: 1, ahead: 2, isReference: false }] as unknown as ShelfRow[];

const baseProps: AttentionRowProps = {
  rows,
  ctx,
  counts,
  sortLabel: 'LAST TOUCHED',
  query: '',
  onQuery: vi.fn(),
};

const draw = (over: Partial<AttentionRowProps> = {}): ReturnType<typeof render> =>
  render(<AttentionRow {...baseProps} {...over} />);

const renderedCountTexts = (): readonly (string | null)[] =>
  [...document.querySelectorAll('.cdt-attention-count')].map((element) => element.textContent);

describe('AttentionRow', () => {
  it('draws exactly the four chips §8.0b names, and no triage copy', () => {
    draw();
    const labels = [...document.querySelectorAll('.cdt-attention-label')].map(
      (element) => element.textContent,
    );
    expect(labels).toEqual(['ALL', 'UNPUSHED', 'UNCOMMITTED', 'COLD']);
    expect(document.body.textContent).not.toContain('UNSORTED');
    expect(document.body.textContent).not.toContain('ALREADY TRIAGED');
  });

  it('sets the chip query rather than filtering itself', () => {
    const onQuery = vi.fn();
    draw({ onQuery });
    screen.getByRole('button', { name: /UNPUSHED/ }).click();
    expect(onQuery).toHaveBeenCalledWith('is:unpushed');
  });

  it('lights the chip whose query is running, compared as an AST', () => {
    draw({ query: 'is:unpushed  ' });
    expect(screen.getByRole('button', { name: /UNPUSHED/ }).getAttribute('aria-pressed')).toBe(
      'true',
    );
    expect(screen.getByRole('button', { name: /COLD/ }).getAttribute('aria-pressed')).toBe('false');
  });

  it('renders the qualified headline from the shared function', () => {
    draw();
    const headline = document.querySelector('.cdt-attention-headline')?.textContent;
    expect(headline).toContain('12 OF 180');
    expect(headline).toContain('CLASSIFIED');
  });

  it('keeps the keyboard hint allowed at --text-5', () => {
    draw();
    expect(screen.getByText(SPACE_PEEK_HINT)).toBeTruthy();
  });

  it('carries the row border itself after §1.7 removed the band above it', () => {
    const { container } = draw();
    expect(container.querySelector('.cdt-attention-row')).toBeTruthy();
  });

  it('never draws a roast because the grid is not that surface', () => {
    draw();
    expect(document.querySelector('.cdt-roast')).toBeNull();
  });

  it('renders every chip count from attentionCounts', () => {
    const projectedCounts = attentionCounts(rows, ctx);
    draw();
    expect(renderedCountTexts()).toEqual(
      ATTENTION_CHIPS.map((chip) => String(projectedCounts[chip.id])),
    );
  });

  it('renders a counted zero for an empty projection, never an empty slot', () => {
    const view = draw({ rows: [] });
    expect(renderedCountTexts()).toEqual(ATTENTION_CHIPS.map(() => '0'));

    view.rerender(<AttentionRow {...baseProps} />);
    const projectedCounts = attentionCounts(rows, ctx);
    expect(renderedCountTexts()).toEqual(
      ATTENTION_CHIPS.map((chip) => String(projectedCounts[chip.id])),
    );
  });

  it('takes every chip label and query from ATTENTION_CHIPS', () => {
    const onQuery = vi.fn();
    draw({ onQuery });
    const labels = [...document.querySelectorAll('.cdt-attention-label')].map(
      (element) => element.textContent,
    );
    expect(labels).toEqual(ATTENTION_CHIPS.map((chip) => chip.label));

    const buttons = screen.getAllByRole('button');
    expect(buttons).toHaveLength(ATTENTION_CHIPS.length);
    ATTENTION_CHIPS.forEach((chip, index) => {
      buttons[index]?.click();
      expect(onQuery).toHaveBeenNthCalledWith(index + 1, chip.query);
    });
  });
});

describe('AttentionChip', () => {
  it('renders no number node for an unknown count but prints a counted zero', () => {
    const unknownView = render(
      <AttentionChip
        label="UNKNOWN"
        count={null}
        subLine="NOT COUNTED"
        pressed={false}
        onActivate={vi.fn()}
      />,
    );
    expect(unknownView.container.querySelector('.cdt-attention-count')).toBeNull();
    expect(unknownView.container.textContent).not.toContain('0');

    const zeroView = render(
      <AttentionChip
        label="ZERO"
        count={0}
        subLine="COUNTED"
        pressed={false}
        onActivate={vi.fn()}
      />,
    );
    expect(zeroView.container.querySelector('.cdt-attention-count')?.textContent).toBe('0');
  });

  it('renders a node in the sub-line', () => {
    const { container } = render(
      <AttentionChip
        label="BROKEN"
        count={null}
        subLine={
          <>
            <s>is:broken</s> dropped
          </>
        }
        pressed={false}
        onActivate={vi.fn()}
      />,
    );
    expect(container.querySelector('s')?.textContent).toBe('is:broken');
  });

  it('defaults the accent and accepts an override', () => {
    const defaultView = render(
      <AttentionChip
        label="DEFAULT"
        count={1}
        subLine="ACCENT"
        pressed={false}
        onActivate={vi.fn()}
      />,
    );
    expect(
      defaultView.container.querySelector('button')?.style.getPropertyValue('--cdt-chip-accent'),
    ).toBe('var(--text-2)');

    const overrideView = render(
      <AttentionChip
        label="OVERRIDE"
        count={1}
        subLine="ACCENT"
        accent="var(--warn)"
        pressed={false}
        onActivate={vi.fn()}
      />,
    );
    expect(
      overrideView.container.querySelector('button')?.style.getPropertyValue('--cdt-chip-accent'),
    ).toBe('var(--warn)');
  });

  it('adds only the broken modifier while retaining the base class', () => {
    const brokenView = render(
      <AttentionChip
        label="BROKEN"
        count={1}
        subLine="STATE"
        broken
        pressed={false}
        onActivate={vi.fn()}
      />,
    );
    const brokenButton = brokenView.container.querySelector('button');
    expect(brokenButton?.classList.contains('cdt-attention-chip')).toBe(true);
    expect(brokenButton?.classList.contains('cdt-attention-chip--broken')).toBe(true);
    expect(brokenButton?.className).toBe('cdt-attention-chip cdt-attention-chip--broken');

    const plainView = render(
      <AttentionChip
        label="PLAIN"
        count={1}
        subLine="STATE"
        pressed={false}
        onActivate={vi.fn()}
      />,
    );
    const plainButton = plainView.container.querySelector('button');
    expect(plainButton?.classList.contains('cdt-attention-chip')).toBe(true);
    expect(plainButton?.classList.contains('cdt-attention-chip--broken')).toBe(false);
    expect(plainButton?.className).toBe('cdt-attention-chip');
  });

  it('activates on click and reflects the pressed state', () => {
    const onActivate = vi.fn();
    const view = render(
      <AttentionChip label="ACTIVE" count={1} subLine="STATE" pressed onActivate={onActivate} />,
    );
    const button = screen.getByRole('button', { name: /ACTIVE/ });
    expect(button.getAttribute('aria-pressed')).toBe('true');
    button.click();
    expect(onActivate).toHaveBeenCalledOnce();

    view.rerender(
      <AttentionChip
        label="ACTIVE"
        count={1}
        subLine="STATE"
        pressed={false}
        onActivate={onActivate}
      />,
    );
    expect(screen.getByRole('button', { name: /ACTIVE/ }).getAttribute('aria-pressed')).toBe(
      'false',
    );
  });
});

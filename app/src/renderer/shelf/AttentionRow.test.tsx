import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ATTENTION_CHIPS, attentionCounts } from './counts.js';
import { AttentionRow, SPACE_PEEK_HINT, type AttentionRowProps } from './AttentionRow.js';
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

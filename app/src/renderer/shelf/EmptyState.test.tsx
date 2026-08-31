import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { parseQuery } from '../../shared/query/parse.js';
import { formatClock } from '../derive/observation.js';
import {
  ADD_ROOT_LABEL,
  CLEAR_QUERY_LABEL,
  EmptyState,
  SCAN_PROBLEMS_LABEL,
  emptyStateModel,
} from './EmptyState.js';

afterEach(cleanup);

const NOW = 1_760_000_000;
const counts = {
  matched: 0,
  total: 180,
  reference: 24,
  classified: 141,
  classificationKnown: true,
};
const emptyCounts = {
  matched: 0,
  total: 0,
  reference: 0,
  classified: 0,
  classificationKnown: false,
};

describe('emptyStateModel', () => {
  it('names the query that ran, not the one that was typed', () => {
    // §8.3 drops a soft-errored term; echoing the raw input names a filter never applied.
    const model = emptyStateModel({
      ast: parseQuery('nosuch:value is:dirty'),
      counts,
      libraryIsEmpty: false,
      now: NOW,
    });
    expect(model.variant).toBe('noMatchWithIgnored');
    expect(model.heading).toBe('NOTHING MATCHES THAT');
    expect(model.reason).toContain('Query: is:dirty');
    expect(model.reason).toContain('180 projects searched');
    expect(model.reason).toContain('24 reference rows excluded');
    expect(model.reason).toContain('1 ignored: nosuch:value');
    expect(model.action).toBe('CLEAR THE QUERY');
  });

  it('never echoes the dropped term as part of the query that ran', () => {
    // The clause names the terms that ran and nothing else. Asserting only that the raw input
    // is absent passes against a line that appends the dropped term after the effective one.
    const model = emptyStateModel({
      ast: parseQuery('nosuch:value is:dirty'),
      counts,
      libraryIsEmpty: false,
      now: NOW,
    });
    const clause = model.reason.slice('Query: '.length, model.reason.indexOf(' · '));
    expect(clause).toBe('is:dirty');
    expect(model.reason.indexOf('nosuch:value')).toBeGreaterThan(model.reason.indexOf('ignored'));
  });

  it('omits the ignored clause when nothing was dropped', () => {
    const model = emptyStateModel({
      ast: parseQuery('is:dirty'),
      counts,
      libraryIsEmpty: false,
      now: NOW,
    });
    expect(model.variant).toBe('noMatch');
    expect(model.reason).not.toMatch(/ignored/);
  });

  it('does not assert that projects exist when the library is empty', () => {
    // The prototype's `<n> projects in the library, none of them shown` reads at n = 0 as a
    // claim that projects exist and are being hidden.
    const model = emptyStateModel({
      ast: parseQuery(''),
      counts: emptyCounts,
      libraryIsEmpty: true,
      now: NOW,
    });
    expect(model.variant).toBe('noLibrary');
    expect(model.heading).toBe('NOTHING INDEXED YET');
    expect(model.reason).toMatch(/^No repositories under the enabled roots as of /);
    expect(model.reason.endsWith('.')).toBe(true);
    // No count at all — not a zero, and not the prototype's claim that rows are being hidden.
    expect(model.reason).not.toMatch(/projects|reference|searched|none of them shown/);
    expect(model.action).toBe('ADD A SCAN ROOT');
  });

  it('states when it looked, so the line claims no currency it does not have', () => {
    const model = emptyStateModel({
      ast: parseQuery(''),
      counts: emptyCounts,
      libraryIsEmpty: true,
      now: NOW,
    });
    expect(model.reason).toContain(formatClock(NOW));
  });

  it('an empty library beats a query, so a filter is never blamed for an empty index', () => {
    const model = emptyStateModel({
      ast: parseQuery('is:dirty'),
      counts: emptyCounts,
      libraryIsEmpty: true,
      now: NOW,
    });
    expect(model.variant).toBe('noLibrary');
    expect(model.action).toBe(ADD_ROOT_LABEL);
  });
});

describe('EmptyState', () => {
  const model = emptyStateModel({
    ast: parseQuery('is:dirty'),
    counts,
    libraryIsEmpty: false,
    now: NOW,
  });
  const empty = emptyStateModel({
    ast: parseQuery(''),
    counts: emptyCounts,
    libraryIsEmpty: true,
    now: NOW,
  });

  it('offers CLEAR THE QUERY where clearing does something', () => {
    render(
      <EmptyState
        model={model}
        onClearQuery={vi.fn()}
        onAddScanRoot={vi.fn()}
        onOpenScanSummary={null}
      />,
    );
    expect(screen.getByRole('button', { name: CLEAR_QUERY_LABEL })).toBeTruthy();
    expect(screen.queryByRole('button', { name: ADD_ROOT_LABEL })).toBeNull();
  });

  it('offers ADD A SCAN ROOT where clearing would clear nothing', () => {
    const onAddScanRoot = vi.fn();
    render(
      <EmptyState
        model={empty}
        onClearQuery={vi.fn()}
        onAddScanRoot={onAddScanRoot}
        onOpenScanSummary={null}
      />,
    );
    expect(screen.queryByRole('button', { name: CLEAR_QUERY_LABEL })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: ADD_ROOT_LABEL }));
    expect(onAddScanRoot).toHaveBeenCalledOnce();
  });

  it('links into the scan summary only when the last run recorded problems', () => {
    const { rerender } = render(
      <EmptyState
        model={empty}
        onClearQuery={vi.fn()}
        onAddScanRoot={vi.fn()}
        onOpenScanSummary={null}
      />,
    );
    expect(screen.queryByRole('button', { name: /problem/i })).toBeNull();
    rerender(
      <EmptyState
        model={empty}
        onClearQuery={vi.fn()}
        onAddScanRoot={vi.fn()}
        onOpenScanSummary={vi.fn()}
      />,
    );
    expect(screen.getByRole('button', { name: SCAN_PROBLEMS_LABEL })).toBeTruthy();
  });

  it('does not link into the scan summary from a filtered shelf', () => {
    // §8.0 grants the link to the empty-library variant. A query that matched nothing is not
    // evidence about the last scan run.
    render(
      <EmptyState
        model={model}
        onClearQuery={vi.fn()}
        onAddScanRoot={vi.fn()}
        onOpenScanSummary={vi.fn()}
      />,
    );
    expect(screen.queryByRole('button', { name: /problem/i })).toBeNull();
  });

  it("paints the reason line at the decision floor, not the prototype's off-token grey", () => {
    const { container } = render(
      <EmptyState
        model={model}
        onClearQuery={vi.fn()}
        onAddScanRoot={vi.fn()}
        onOpenScanSummary={null}
      />,
    );
    expect(container.querySelector('.cdt-shelf-empty-reason')).toBeTruthy();
    expect(container.innerHTML).not.toMatch(/#7a8896/);
    expect(container.innerHTML).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });

  it('originates no path and names no destructive operation', () => {
    const { container } = render(
      <EmptyState
        model={model}
        onClearQuery={vi.fn()}
        onAddScanRoot={vi.fn()}
        onOpenScanSummary={null}
      />,
    );
    expect(container.innerHTML).not.toMatch(/FORGET/i);
    // `roots.add` is privileged and takes its path from the shell's native dialog; this
    // component constructs none and shows none.
    expect(container.innerHTML).not.toMatch(/[A-Za-z]:\\|\/home\/|\/mnt\//);
  });
});

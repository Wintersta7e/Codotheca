import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ComponentProps, ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Collection, CollectionId, ProjectId } from '../../generated/protocol.js';
import type { QueryAst, QueryTerm } from '../../shared/query/ast.js';
import { makeProjectRow } from '../testing/projectRow.js';
import { CollectionChipRow, savedChipOrder } from './CollectionChipRow.js';
import { fakeEngine } from './engine.js';
import { CollectionsProvider, useCollections, type CollectionsApi } from './store.js';

afterEach(cleanup);

/**
 * The store settles in a microtask, and `waitFor` succeeds on its first check — so
 * `await waitFor(() => expect(queryByRole('button')).toBeNull())` passes on the *loading*
 * render and never observes the one it is about. Measured: with this probe absent, deleting
 * the row's hydration guard left every assertion green. The probe makes the settled state
 * observable so the assertion is made against the render that could actually render a chip.
 */
function StoreStatus(): ReactElement {
  return <span data-testid="store">{useCollections().state.status}</span>;
}

const settled = async (status: 'ready' | 'unavailable'): Promise<void> => {
  await waitFor(() => {
    expect(screen.getByTestId('store').textContent).toBe(status);
  });
};

const saved = (id: number, name: string, sortIndex: number): Collection => ({
  id: id as CollectionId,
  name,
  kind: 'query',
  queryText: `q${String(id)}`,
  queryGrammarVersion: 1,
  sortIndex,
  memberCount: null,
});

const rows = [makeProjectRow({ id: 1 as ProjectId }), makeProjectRow({ id: 2 as ProjectId })];
const engine = fakeEngine({ filter: (_a, all) => all });

const textOf = (term: QueryTerm): string => (term.kind === 'bare' ? term.text : '');
const sortedCanonical = (ast: QueryAst): string => ast.terms.map(textOf).sort().join(' ');

const mount = (
  collections: readonly Collection[],
  over: Partial<ComponentProps<typeof CollectionChipRow>> = {},
  api: Partial<CollectionsApi> = {},
): void => {
  render(
    <CollectionsProvider
      api={{ list: () => Promise.resolve(collections), create: vi.fn(), remove: vi.fn(), ...api }}
    >
      <StoreStatus />
      <CollectionChipRow
        rows={rows}
        currentQuery=""
        engine={engine}
        nowMs={() => 0}
        onQueryChange={vi.fn()}
        {...over}
      />
    </CollectionsProvider>,
  );
};

describe('savedChipOrder', () => {
  it('is sort_index order, appended after the four built-ins by the caller', () => {
    const out = savedChipOrder([saved(1, 'c', 5), saved(2, 'a', 1), saved(3, 'b', 3)]);
    expect(out.map((c) => c.name)).toEqual(['a', 'b', 'c']);
  });

  it('does not reorder its input in place', () => {
    const input = [saved(1, 'c', 5), saved(2, 'a', 1)];
    savedChipOrder(input);
    expect(input.map((c) => c.name)).toEqual(['c', 'a']);
  });
});

describe('CollectionChipRow', () => {
  // Never render unknown as zero: a count over an unhydrated projection is a false measurement.
  it('renders nothing at all before the projection has hydrated', async () => {
    mount([saved(1, 'Rust work', 0)], { rows: null });
    // The list has arrived and there is a collection to draw. What is missing is the
    // projection, and a chip drawn now would carry a count of zero nobody measured.
    await settled('ready');
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('renders nothing while the collection list is still loading', () => {
    mount([saved(1, 'Rust work', 0)], {}, { list: () => new Promise(() => undefined) });
    expect(screen.getByTestId('store').textContent).toBe('loading');
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('renders nothing when the list could not be read', async () => {
    mount([saved(1, 'Rust work', 0)], {}, { list: () => Promise.reject(new Error('NO_CORE')) });
    await settled('unavailable');
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('renders every saved chip and truncates none of them', async () => {
    const many = Array.from({ length: 16 }, (_, i) => saved(i + 1, `c${String(i)}`, i));
    mount(many);
    await waitFor(() => {
      expect(screen.getAllByRole('button', { name: /^c\d+, / })).toHaveLength(16);
    });
  });

  it('activating writes the collection’s query into the field', async () => {
    const onQueryChange = vi.fn();
    mount([saved(1, 'Rust work', 0)], { onQueryChange });
    fireEvent.click(await screen.findByRole('button', { name: /^Rust work/ }));
    expect(onQueryChange).toHaveBeenCalledWith('q1');
  });

  it('lights the chip whose canonical AST equals the field, whatever the term order', async () => {
    const ordered = fakeEngine({ filter: (_a, all) => all, canonical: sortedCanonical });
    mount([{ ...saved(1, 'Rust work', 0), queryText: 'lang:rust is:dirty' }], {
      engine: ordered,
      currentQuery: 'is:dirty lang:rust',
    });
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^Rust work/ }).getAttribute('aria-pressed')).toBe(
        'true',
      );
    });
  });

  // §8.8: deleting a name is not deleting work.
  it('two presses remove the collection and leave the query exactly as it was', async () => {
    const remove = vi.fn(() => Promise.resolve());
    const onQueryChange = vi.fn();
    mount([saved(1, 'Rust work', 0)], { currentQuery: 'q1', onQueryChange }, { remove });
    fireEvent.click(await screen.findByRole('button', { name: 'Remove the collection Rust work' }));
    expect(remove).not.toHaveBeenCalled();
    expect(screen.getByText('PRESS AGAIN TO DELETE · THE PROJECTS STAY')).toBeTruthy();
    fireEvent.click(
      screen.getByRole('button', { name: /^Remove the collection Rust work\. Press again/ }),
    );
    await waitFor(() => {
      expect(remove).toHaveBeenCalledWith(1);
    });
    expect(onQueryChange).not.toHaveBeenCalled();
  });

  it('arming a second chip disarms the first', async () => {
    mount([saved(1, 'One', 0), saved(2, 'Two', 1)]);
    fireEvent.click(await screen.findByRole('button', { name: 'Remove the collection One' }));
    fireEvent.click(screen.getByRole('button', { name: 'Remove the collection Two' }));
    expect(screen.getAllByText('PRESS AGAIN TO DELETE · THE PROJECTS STAY')).toHaveLength(1);
  });
});

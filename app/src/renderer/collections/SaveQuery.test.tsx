import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ComponentProps, ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Collection, CollectionId } from '../../generated/protocol.js';
import {
  CollectionNameInput,
  SAVE_CONTROL_NAME,
  SaveQueryButton,
  useSaveQueryFlow,
  type SaveQueryDeps,
} from './SaveQuery.js';
import { fakeEngine } from './engine.js';
import { CollectionsProvider, useCollections, type CollectionsApi } from './store.js';

afterEach(cleanup);

function StoreStatus(): ReactElement {
  return <span data-testid="store-status">{useCollections().state.status}</span>;
}

function SaveQueryHarness(props: SaveQueryDeps): ReactElement {
  const flow = useSaveQueryFlow(props);
  return (
    <>
      <CollectionNameInput flow={flow} />
      <SaveQueryButton flow={flow} />
    </>
  );
}

function saved(overrides: Partial<Collection> = {}): Collection {
  return {
    id: 1 as CollectionId,
    name: 'Existing collection',
    kind: 'query',
    queryText: 'is:dirty',
    queryGrammarVersion: 1,
    sortIndex: 0,
    memberCount: null,
    ...overrides,
  };
}

function mount(
  collections: readonly Collection[] = [],
  props: Partial<ComponentProps<typeof SaveQueryHarness>> = {},
  api: Partial<CollectionsApi> = {},
): void {
  render(
    <CollectionsProvider
      api={{
        list: () => Promise.resolve(collections),
        create: () => Promise.resolve(null),
        remove: () => Promise.resolve(),
        ...api,
      }}
    >
      <StoreStatus />
      <SaveQueryHarness
        queryText={props.queryText ?? 'lang:rust'}
        engine={props.engine ?? fakeEngine()}
        onRestoreQuery={props.onRestoreQuery ?? vi.fn()}
      />
    </CollectionsProvider>,
  );
}

async function waitUntilReady(): Promise<void> {
  await waitFor(() => {
    expect(screen.getByTestId('store-status').textContent).toBe('ready');
  });
}

function saveControl(): HTMLButtonElement {
  const control = screen.getByRole('button', { name: SAVE_CONTROL_NAME });
  if (!(control instanceof HTMLButtonElement)) throw new Error('save control is not a button');
  return control;
}

function nameInput(): HTMLInputElement {
  const input = screen.getByRole('textbox', { name: 'Collection name' });
  if (!(input instanceof HTMLInputElement)) throw new Error('name field is not an input');
  return input;
}

describe('SaveQuery', () => {
  it('renders SAVE for a non-empty query with no chip of its own', async () => {
    mount();
    await waitUntilReady();

    await waitFor(() => {
      expect(saveControl().textContent).toBe('SAVE');
    });
  });

  it('renders nothing for a whitespace-only query', async () => {
    mount([], { queryText: ' \t  ' });
    await waitUntilReady();

    await waitFor(() => {
      expect(screen.queryByRole('button', { name: SAVE_CONTROL_NAME })).toBeNull();
    });
  });

  it('renders nothing when the query already equals a saved collection query', async () => {
    mount([saved({ queryText: 'lang:rust' })]);
    await waitUntilReady();

    await waitFor(() => {
      expect(screen.queryByRole('button', { name: SAVE_CONTROL_NAME })).toBeNull();
    });
  });

  it('pre-fills the name from the AST and fully selects it', async () => {
    mount([], { queryText: '  is:dirty    lang:rust ' });
    await waitUntilReady();
    fireEvent.click(saveControl());
    const input = nameInput();

    await waitFor(() => {
      expect(input.value).toBe('is:dirty lang:rust');
      expect(input.selectionStart).toBe(0);
      expect(input.selectionEnd).toBe(input.value.length);
    });
  });

  it('commits on Enter with max sort index plus one and the grammar version', async () => {
    const create: CollectionsApi['create'] = vi.fn(() => Promise.resolve(null));
    const engine = fakeEngine({ grammarVersion: 7 });
    mount(
      [saved({ sortIndex: 6 })],
      { engine },
      {
        create,
      },
    );
    await waitUntilReady();
    fireEvent.click(saveControl());
    const input = nameInput();
    fireEvent.change(input, { target: { value: 'Rust work' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() => {
      expect(create).toHaveBeenCalledWith({
        name: 'Rust work',
        queryText: 'lang:rust',
        queryGrammarVersion: engine.grammarVersion,
        sortIndex: 7,
      });
      expect(screen.queryByRole('textbox', { name: 'Collection name' })).toBeNull();
    });
  });

  it('restores the original query byte for byte on Escape and closes the field', async () => {
    const original = '  is:dirty    lang:rust ';
    const onRestoreQuery = vi.fn();
    mount([], { queryText: original, onRestoreQuery });
    await waitUntilReady();
    fireEvent.click(saveControl());
    fireEvent.keyDown(nameInput(), { key: 'Escape' });

    await waitFor(() => {
      expect(onRestoreQuery).toHaveBeenLastCalledWith(original);
      expect(screen.queryByRole('textbox', { name: 'Collection name' })).toBeNull();
    });
  });

  it('refuses an empty name in the control without calling create', async () => {
    const create: CollectionsApi['create'] = vi.fn(() => Promise.resolve(null));
    mount([], {}, { create });
    await waitUntilReady();
    fireEvent.click(saveControl());
    const input = nameInput();
    fireEvent.change(input, { target: { value: '' } });
    fireEvent.keyDown(input, { key: 'Enter' });

    await waitFor(() => {
      expect(saveControl().textContent).toBe('NAME IT');
      expect(input.getAttribute('aria-invalid')).toBe('true');
      expect(create).not.toHaveBeenCalled();
    });
  });

  it('clears the refusal as soon as the name changes', async () => {
    mount();
    await waitUntilReady();
    fireEvent.click(saveControl());
    const input = nameInput();
    fireEvent.change(input, { target: { value: '' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    fireEvent.change(input, { target: { value: 'R' } });

    await waitFor(() => {
      expect(saveControl().textContent).toBe('SAVE');
    });
  });

  it('marks the save control as a polite live region', async () => {
    mount();
    await waitUntilReady();

    await waitFor(() => {
      expect(saveControl().getAttribute('aria-live')).toBe('polite');
    });
  });

  it('keeps the flow open and shows a refusal returned by the core', async () => {
    const create: CollectionsApi['create'] = vi.fn(() =>
      Promise.resolve<'name_taken'>('name_taken'),
    );
    const onRestoreQuery = vi.fn();
    mount([], { onRestoreQuery }, { create });
    await waitUntilReady();
    fireEvent.click(saveControl());
    fireEvent.keyDown(nameInput(), { key: 'Enter' });

    await waitFor(() => {
      expect(saveControl().textContent).toBe('THAT NAME IS TAKEN');
      expect(screen.queryByRole('textbox', { name: 'Collection name' })).not.toBeNull();
      expect(onRestoreQuery).not.toHaveBeenCalled();
    });
  });

  // A request that never reaches the core is not a refusal and §8.8 has no wording for it. The
  // field keeps the typed name so `Enter` can be pressed again, and nothing escapes as an
  // unhandled rejection — which in the renderer is a crash, not a message.
  it('keeps the typed name and raises nothing when the request fails outright', async () => {
    const onRestoreQuery = vi.fn();
    mount([], { onRestoreQuery }, { create: () => Promise.reject(new Error('CORE_UNAVAILABLE')) });
    await waitUntilReady();
    fireEvent.click(saveControl());
    fireEvent.change(nameInput(), { target: { value: 'Rust work' } });
    fireEvent.keyDown(nameInput(), { key: 'Enter' });

    // Two ticks, so the rejection has settled before anything is asserted. Vitest fails a file
    // on an unhandled rejection, so an uncaught one here reddens this test rather than escaping
    // silently — which is what it would do in the renderer, as a crash and not a message.
    await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(nameInput().value).toBe('Rust work');
    expect(onRestoreQuery).not.toHaveBeenCalled();
  });

  it('restores the query and closes the field after a successful commit', async () => {
    const create: CollectionsApi['create'] = vi.fn(() => Promise.resolve(null));
    const onRestoreQuery = vi.fn();
    mount([], { onRestoreQuery }, { create });
    await waitUntilReady();
    fireEvent.click(saveControl());
    fireEvent.keyDown(nameInput(), { key: 'Enter' });

    await waitFor(() => {
      expect(onRestoreQuery).toHaveBeenCalledWith('lang:rust');
      expect(screen.queryByRole('textbox', { name: 'Collection name' })).toBeNull();
    });
  });
});

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Collection, CollectionId, ProjectId } from '../../generated/protocol.js';
import type { QueryAst, QueryTerm } from '../../shared/query/ast.js';
import { makeProjectRow } from '../testing/projectRow.js';
import { CollectionChip, REMOVE_GLYPH, removeControlName } from './CollectionChip.js';
import { collectionChipModel } from './chipModel.js';
import { fakeEngine } from './engine.js';

afterEach(cleanup);

const saved = (over: Partial<Collection> = {}): Collection => ({
  id: 7 as CollectionId,
  name: 'Rust work',
  kind: 'query',
  queryText: 'lang:rust is:dirty',
  queryGrammarVersion: 1,
  sortIndex: 0,
  memberCount: null,
  ...over,
});

const rows = [makeProjectRow({ id: 1 as ProjectId }), makeProjectRow({ id: 2 as ProjectId })];

// R13: plan 13's own `QueryAst`, built rather than cast.
const ast = (...texts: readonly string[]): QueryAst => ({
  grammarVersion: 1,
  terms: texts.map((text): QueryTerm => ({ kind: 'bare', negated: false, text })),
  ignored: [],
});

const passthrough = fakeEngine({ filter: (_a, all) => all });

const chip = (
  collection: Collection,
  engine = passthrough,
  over: Partial<ComponentProps<typeof CollectionChip>> = {},
): ReturnType<typeof vi.fn> => {
  const onPressRemove = vi.fn();
  render(
    <CollectionChip
      model={collectionChipModel(collection, rows, engine)}
      active={false}
      armed={false}
      onActivate={vi.fn()}
      onPressRemove={onPressRemove}
      onDisarm={vi.fn()}
      {...over}
    />,
  );
  return onPressRemove;
};

describe('CollectionChip', () => {
  it('is a button named by what it counted, never by its colour', () => {
    chip(saved());
    const button = screen.getByRole('button', { name: 'Rust work, 2 projects' });
    expect(button.getAttribute('aria-pressed')).toBe('false');
    expect(button.textContent).toContain('RUST WORK');
    expect(button.textContent).toContain('lang:rust is:dirty');
  });

  it('activates by handing back the collection’s query', () => {
    const onActivate = vi.fn();
    chip(saved(), passthrough, { onActivate });
    fireEvent.click(screen.getByRole('button', { name: /^Rust work/ }));
    expect(onActivate).toHaveBeenCalledWith('lang:rust is:dirty');
  });

  it('reports aria-pressed when the field already states the scope', () => {
    chip(saved(), passthrough, { active: true });
    expect(screen.getByRole('button', { name: /^Rust work/ }).getAttribute('aria-pressed')).toBe(
      'true',
    );
  });

  it('strikes the dropped terms and keeps the survivors plain', () => {
    const engine = fakeEngine({
      parse: () => ({
        ast: ast('lang:rust'),
        dropped: [{ text: 'sparkle:yes', reason: 'unknownField' }],
      }),
      canonical: () => 'lang:rust',
      filter: (_a, all) => all,
    });
    chip(saved(), engine);
    expect(screen.getByText('sparkle:yes').classList.contains('cdt-collection-dropped')).toBe(true);
    expect(screen.getByText('lang:rust').classList.contains('cdt-collection-dropped')).toBe(false);
  });

  // §8.8: broken renders no number at all — not 0, not an em dash, not a dimmed digit.
  it('renders no number node at all when broken', () => {
    const { container } = render(
      <CollectionChip
        model={collectionChipModel(
          saved({ queryGrammarVersion: 9 }),
          rows,
          fakeEngine({ grammarVersion: 1 }),
        )}
        active={false}
        armed={false}
        onActivate={vi.fn()}
        onPressRemove={vi.fn()}
        onDisarm={vi.fn()}
      />,
    );
    expect(container.querySelector('[data-part="count"]')).toBeNull();
    expect(container.textContent).not.toMatch(/\d/);
    expect(container.textContent).not.toContain('—');
    const button = screen.getByRole('button', { name: /^Rust work/ });
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });

  it('marks the wrapper with the state the stylesheet selects on', () => {
    const { container } = render(
      <CollectionChip
        model={collectionChipModel(
          saved({ queryGrammarVersion: 9 }),
          rows,
          fakeEngine({ grammarVersion: 1 }),
        )}
        active={false}
        armed
        onActivate={vi.fn()}
        onPressRemove={vi.fn()}
        onDisarm={vi.fn()}
      />,
    );
    const wrapper = container.querySelector('.cdt-collection');
    expect(wrapper?.getAttribute('data-state')).toBe('broken');
    expect(wrapper?.getAttribute('data-armed')).toBe('true');
    expect(wrapper?.getAttribute('data-active')).toBe('false');
  });

  it('carries a × that names what it removes and never how it looks', () => {
    const onPressRemove = chip(saved());
    const remove = screen.getByRole('button', { name: 'Remove the collection Rust work' });
    expect(remove.textContent).toBe(REMOVE_GLYPH);
    fireEvent.click(remove);
    expect(onPressRemove).toHaveBeenCalledWith(7);
  });

  it('Delete on a focused chip is the same action as the ×', () => {
    const onPressRemove = chip(saved());
    fireEvent.keyDown(screen.getByRole('button', { name: /^Rust work/ }), { key: 'Delete' });
    expect(onPressRemove).toHaveBeenCalledWith(7);
  });

  it('armed states in words what deletion does not touch, and renames the ×', () => {
    chip(saved(), passthrough, { armed: true });
    expect(screen.getByText('PRESS AGAIN TO DELETE · THE PROJECTS STAY')).toBeTruthy();
    expect(
      screen.getByRole('button', {
        name: 'Remove the collection Rust work. Press again to confirm. The projects stay.',
      }),
    ).toBeTruthy();
  });

  it('Esc disarms and does not reach the shelf’s close-peek binding', () => {
    const onDisarm = vi.fn();
    const outer = vi.fn();
    render(
      <div onKeyDown={outer}>
        <CollectionChip
          model={collectionChipModel(saved(), rows, passthrough)}
          active={false}
          armed
          onActivate={vi.fn()}
          onPressRemove={vi.fn()}
          onDisarm={onDisarm}
        />
      </div>,
    );
    fireEvent.keyDown(screen.getByRole('button', { name: /^Rust work/ }), { key: 'Escape' });
    expect(onDisarm).toHaveBeenCalled();
    expect(outer).not.toHaveBeenCalled();
  });

  // Unarmed, `Esc` is the shelf's — closing Peek. Swallowing it here would make a chip anywhere
  // in the row a place where that binding silently dies.
  it('lets Esc through while nothing is armed', () => {
    const onDisarm = vi.fn();
    const outer = vi.fn();
    render(
      <div onKeyDown={outer}>
        <CollectionChip
          model={collectionChipModel(saved(), rows, passthrough)}
          active={false}
          armed={false}
          onActivate={vi.fn()}
          onPressRemove={vi.fn()}
          onDisarm={onDisarm}
        />
      </div>,
    );
    fireEvent.keyDown(screen.getByRole('button', { name: /^Rust work/ }), { key: 'Escape' });
    expect(onDisarm).not.toHaveBeenCalled();
    expect(outer).toHaveBeenCalled();
  });

  it('names the × without the word this product has banned', () => {
    expect(removeControlName('Rust work', false)).not.toMatch(/FORGET/i);
    expect(removeControlName('Rust work', true)).toContain('The projects stay.');
  });
});

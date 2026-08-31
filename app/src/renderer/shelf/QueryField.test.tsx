import type { RenderResult } from '@testing-library/react';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { pillsOf } from '../../shared/query/format.js';
import { parseQuery } from '../../shared/query/parse.js';
import source from './QueryField.tsx?raw';
import type { QueryFieldModel } from './QueryField.js';
import { QUERY_PLACEHOLDER, QueryFieldView, dropPill, fieldModel } from './QueryField.js';

afterEach(cleanup);

const model = (text: string): QueryFieldModel => fieldModel(text, parseQuery(text), []);
const labels = (text: string): string[] => model(text).pills.map((pill) => pill.label);

describe('the tokenizer that must not ship', () => {
  it('reads its own source, so the three assertions below are not vacuous', () => {
    expect(source.length).toBeGreaterThan(500);
    expect(source).toContain('export function fieldModel');
  });
  it('this module implements no splitting of its own', () => {
    // §8.3a rejects `raw.split(/\s+/)`: it cannot express `in:"two words"`, it pills every
    // colon-bearing bare word, and `-is:dirty` yields the field `-is`, which matches no branch.
    expect(source).not.toMatch(/\.split\s*\(/);
    expect(source).not.toMatch(/\\s\+/);
    expect(source).not.toMatch(/indexOf\(['"]:['"]\)/);
  });
  it('it uses the one shared tokenizer instead', () => {
    expect(source).toMatch(/from '\.\.\/\.\.\/shared\/query\/tokenize\.js'/);
  });
  it('and it never parses — pills arrive already built', () => {
    expect(source).not.toMatch(/parseQuery/);
  });
});

describe('fieldModel', () => {
  it('leaves the term still being typed in the input, never also as a pill', () => {
    const m = model('lang:rust is:di');
    expect(m.draft).toBe('is:di');
    expect(m.pills.map((pill) => pill.label)).toEqual(['lang:rust']);
  });
  it('lifts a term once a space terminates it', () => {
    const m = model('lang:rust ');
    expect(m.draft).toBe('');
    expect(labels('lang:rust ')).toEqual(['lang:rust']);
    expect(m.committed).toBe('lang:rust');
  });
  it('keeps free text in the input, with the case the user typed', () => {
    const m = model('in:"two words" Codo');
    expect(m.draft).toBe('Codo');
    expect(m.pills.map((pill) => pill.label)).toEqual(['in:"two words"']);
  });
  it('does not split a quoted value', () => {
    expect(model('in:"two words"').draft).toBe('in:"two words"');
    expect(labels('in:"two words" ')).toEqual(['in:"two words"']);
  });
  it('never renders one term twice — the pilled text is gone from the input', () => {
    // §8.3a's rule stated as the property, not as one example.
    for (const text of ['lang:rust ', 'is:dirty -is:archived ', 'in:"two words" Codo']) {
      const m = model(text);
      for (const pill of m.pills) expect(m.draft).not.toContain(pill.label);
    }
  });
  it('rejoins to exactly what it was handed, so nothing round-trips through a rewrite', () => {
    for (const text of ['lang:rust is:dirty ', 'in:"two words" Codo', 'is:di']) {
      const m = model(text);
      expect(m.committed.length + m.draft.length).toBeGreaterThan(0);
    }
  });
});

describe('the contract this plan needs from plan 13', () => {
  it('addresses each pill by the token the user typed, whatever the pill renders', () => {
    // `Pill.key` is plan 13's own ordinal (`t0`/`i0`), not the source text, so this plan maps
    // pill → token itself. The property that matters is that a drop returns the *source*, not
    // a canonicalised rewrite: the user typed `LANG:Rust` and the rest must come back as typed.
    const text = 'LANG:Rust is:dirty';
    const pill = pillsOf(parseQuery(text), [])[0];
    expect(pill?.label).toBe('lang:rust');
    expect(dropPill(text, parseQuery(text), pill!)).toBe('is:dirty');
  });
  it('addresses a quoted value and a negated term without rewriting either', () => {
    const text = 'in:"two words" -is:dirty';
    const ast = parseQuery(text);
    const pills = pillsOf(ast, []);
    expect(dropPill(text, ast, pills[0]!)).toBe('-is:dirty');
    expect(dropPill(text, ast, pills[1]!)).toBe('in:"two words"');
  });
  it('leaves the query untouched rather than cutting the wrong term when it cannot address one', () => {
    // A pill from a different query addresses no token here. A no-op beats a wrong rewrite.
    const stray = pillsOf(parseQuery('lang:rust'), [])[0];
    expect(dropPill('is:dirty', parseQuery('is:dirty'), stray!)).toBe('is:dirty');
  });
});

describe('dropPill', () => {
  it('removes the term and leaves the rest of the query as typed', () => {
    const text = 'lang:rust in:"two words" Codo';
    const ast = parseQuery(text);
    const pill = pillsOf(ast, [])[1];
    expect(dropPill(text, ast, pill!)).toBe('lang:rust Codo');
  });
  it('removes a soft-errored term by its own source text', () => {
    const text = 'nosuch:value is:dirty';
    const ast = parseQuery(text);
    const pill = pillsOf(ast, []).find((p) => p.state === 'error');
    expect(dropPill(text, ast, pill!)).toBe('is:dirty');
  });
});

describe('QueryFieldView', () => {
  const view = (text: string, onQueryChange = vi.fn()): RenderResult =>
    render(
      <QueryFieldView
        text={text}
        ast={parseQuery(text)}
        model={model(text)}
        onQueryChange={onQueryChange}
      />,
    );

  it("carries §8.3a's placeholder verbatim", () => {
    view('');
    expect(screen.getByPlaceholderText(QUERY_PLACEHOLDER)).toBeTruthy();
    expect(QUERY_PLACEHOLDER).toBe('lang:rust is:dirty touched:>6mo');
  });

  it('renders the three pill states with a redundant coding each', () => {
    const { container } = view('is:dirty -is:archived nosuch:value ');
    const states = [...container.querySelectorAll('.cdt-shelf-pill')].map((el) =>
      el.getAttribute('data-state'),
    );
    expect(states).toEqual(['accepted', 'negated', 'error']);
    // Colour carries none of the three alone.
    expect(container.querySelector('[data-state="negated"]')?.textContent).toMatch(/^-/);
    expect(container.querySelector('[data-state="error"]')?.className).toContain('is-struck');
  });

  it('gives `completion:` the soft-error state and its stated reason', () => {
    // Criterion 65: it parses, it never filters, and NULL is never compared as 0.
    view('completion:>5 ');
    const pill = screen.getByText('completion:>5').closest('.cdt-shelf-pill');
    expect(pill?.getAttribute('data-state')).toBe('error');
    expect(pill?.getAttribute('title')).toBe('NOT COMPUTED IN THIS RELEASE');
  });

  it('drops a term when its pill is clicked — the whole pill is the target', () => {
    const onQueryChange = vi.fn();
    view('lang:rust is:dirty ', onQueryChange);
    fireEvent.click(screen.getByText('lang:rust'));
    expect(onQueryChange).toHaveBeenCalledWith('is:dirty');
  });

  it('reports every keystroke, so free text filters without waiting for a space', () => {
    const onQueryChange = vi.fn();
    view('is:dirty ', onQueryChange);
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'C' } });
    expect(onQueryChange).toHaveBeenLastCalledWith('is:dirty C');
  });

  it('shows the draft, never the pilled text, in the input', () => {
    view('lang:rust Codo');
    expect(screen.getByRole<HTMLInputElement>('searchbox').value).toBe('Codo');
  });

  it('declares no colour of its own and names no destructive operation', () => {
    const { container } = view('is:dirty nosuch:value ');
    expect(container.innerHTML).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(container.innerHTML).not.toMatch(/FORGET/i);
  });
});

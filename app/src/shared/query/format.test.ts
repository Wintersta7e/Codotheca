import { describe, expect, it } from 'vitest';
import type { QueryTerm } from './ast.js';
import { parseQuery } from './parse.js';
import {
  effectiveQueryText,
  freeTextOf,
  ignoredClause,
  ignoredReasonLabel,
  pillsOf,
  renderTerm,
} from './format.js';

/** The one term `query` parses to. Throws rather than asserting non-null, so a parser
 *  regression names the query it broke instead of failing on an undefined property. */
function only(query: string): QueryTerm {
  const [term] = parseQuery(query).terms;
  if (term === undefined) throw new Error(`no term parsed from ${query}`);
  return term;
}

describe('renderTerm', () => {
  it('round-trips every branch back to canonical source text', () => {
    for (const query of [
      'lang:rust',
      'owner:someone',
      'in:"two words"',
      'collection:"side projects"',
      'is:dirty',
      '-is:dirty',
      'has:stash',
      'size:>10mb',
      'touched:>6mo',
      'touched:2019',
    ]) {
      expect(renderTerm(only(query)), query).toBe(query);
    }
  });
  it('re-quotes a value that needs it', () => {
    expect(renderTerm(only('in:"two words"'))).toBe('in:"two words"');
  });
});

describe('effectiveQueryText', () => {
  it('prints the terms that ran and not the ones that were dropped', () => {
    const ast = parseQuery('completion:>5 nosuch:x lang:rust');
    expect(effectiveQueryText(ast)).toBe('lang:rust');
  });
  it('is empty for a query whose every term was dropped', () => {
    expect(effectiveQueryText(parseQuery('completion:>5'))).toBe('');
  });
});

describe('pillsOf', () => {
  it('gives an accepted field term an accepted pill', () => {
    expect(pillsOf(parseQuery('lang:rust'), [])).toEqual([
      { key: 't0', label: 'lang:rust', state: 'accepted', reason: null },
    ]);
  });
  it('gives a negated term the negated state and keeps the dash in the label', () => {
    expect(pillsOf(parseQuery('-is:dirty'), [])).toEqual([
      { key: 't0', label: '-is:dirty', state: 'negated', reason: null },
    ]);
  });
  it('gives a soft-errored term the error state and its reason', () => {
    expect(pillsOf(parseQuery('completion:>5'), [])).toEqual([
      { key: 'i0', label: 'completion:>5', state: 'error', reason: 'NOT COMPUTED IN THIS RELEASE' },
    ]);
  });
  it('does not pill free text — the input holds it, and the pill text is never rendered twice', () => {
    expect(pillsOf(parseQuery('codo lang:rust'), [])).toHaveLength(1);
  });
  it('appends terms the projection cannot answer', () => {
    const pills = pillsOf(parseQuery('lang:rust'), [{ text: 'has:ci', reason: 'notAvailable' }]);
    expect(pills.at(-1)).toEqual({
      key: 'x0',
      label: 'has:ci',
      state: 'error',
      reason: 'NOT AVAILABLE IN THIS RELEASE',
    });
  });
});

describe('freeTextOf', () => {
  it('returns the bare terms only, space-joined', () => {
    expect(freeTextOf(parseQuery('codo lang:rust theca'))).toBe('codo theca');
  });
});

describe('ignoredClause', () => {
  it('is empty when nothing was ignored', () => {
    expect(ignoredClause([])).toBe('');
  });
  it('names the count and the terms, §8.0 row two', () => {
    expect(
      ignoredClause([
        { text: 'completion:>5', reason: 'notComputed' },
        { text: 'nosuch:x', reason: 'unknownField' },
      ]),
    ).toBe(' · 2 ignored: completion:>5, nosuch:x');
  });
});

describe('ignoredReasonLabel', () => {
  it("names each reason in the pill's own words", () => {
    expect(ignoredReasonLabel('notComputed')).toBe('NOT COMPUTED IN THIS RELEASE');
    expect(ignoredReasonLabel('unknownField')).toBe('NOT A FIELD');
    expect(ignoredReasonLabel('malformedValue')).toBe('NOT A VALUE FOR THIS FIELD');
    expect(ignoredReasonLabel('notAvailable')).toBe('NOT AVAILABLE IN THIS RELEASE');
  });
});

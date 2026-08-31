import { describe, expect, it } from 'vitest';
import { splitFieldValue, tokenizeQuery, unquote } from './tokenize.js';

const texts = (input: string): string[] => tokenizeQuery(input).map((t) => t.text);

describe('tokenizeQuery', () => {
  it('splits on whitespace', () => {
    expect(texts('lang:rust is:dirty')).toEqual(['lang:rust', 'is:dirty']);
  });
  it('keeps a quoted value whole after a colon', () => {
    // The exact case §8.3a says the design tokenizer cannot express at all.
    expect(texts('in:"two words" is:dirty')).toEqual(['in:"two words"', 'is:dirty']);
  });
  it('keeps a wholly quoted bare term whole', () => {
    expect(texts('"two words"')).toEqual(['"two words"']);
  });
  it('honours \\" as the only escape', () => {
    expect(texts('owner:"a \\" b"')).toEqual(['owner:"a \\" b"']);
  });
  it('keeps a leading dash attached to its term', () => {
    expect(texts('-is:dirty')).toEqual(['-is:dirty']);
  });
  it('drops runs of whitespace and returns nothing for an empty query', () => {
    expect(texts('   ')).toEqual([]);
  });
  it("reports each token's span so a pill can address its own source text", () => {
    const [first, second] = tokenizeQuery('ab cd');
    expect(first).toEqual({ text: 'ab', start: 0, end: 2 });
    expect(second).toEqual({ text: 'cd', start: 3, end: 5 });
  });
  it('tolerates an unterminated quote by taking the rest of the input', () => {
    expect(texts('in:"unterminated')).toEqual(['in:"unterminated']);
  });
});

describe('splitFieldValue', () => {
  it('splits at the first colon', () => {
    expect(splitFieldValue('lang:rust')).toEqual({ field: 'lang', value: 'rust', quoted: false });
  });
  it('unquotes the value and says it was quoted', () => {
    expect(splitFieldValue('in:"two words"')).toEqual({
      field: 'in',
      value: 'two words',
      quoted: true,
    });
  });
  it('returns null when there is no colon', () => {
    expect(splitFieldValue('rust')).toBeNull();
  });
  it('returns null when the colon leads', () => {
    expect(splitFieldValue(':rust')).toBeNull();
  });
  it('keeps a colon inside the value', () => {
    expect(splitFieldValue('in:wsl:ubuntu')).toEqual({
      field: 'in',
      value: 'wsl:ubuntu',
      quoted: false,
    });
  });
});

describe('unquote', () => {
  it('resolves the one escape', () => {
    expect(unquote('"a \\" b"')).toEqual({ value: 'a " b', quoted: true });
  });
  it('leaves an unquoted value alone', () => {
    expect(unquote('rust')).toEqual({ value: 'rust', quoted: false });
  });
});

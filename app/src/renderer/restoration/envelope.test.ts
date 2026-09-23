import { describe, expect, it } from 'vitest';
import vocabularyRaw from '../styles/css-vocabulary.json?raw';
import { RESTORATION_SURGE_MS } from './envelope';

/**
 * §11.6 owns the restoration envelope, and `check-style-tokens.mjs` enforces §11.6 against the
 * stylesheet through `css-vocabulary.json`. The constant the renderer reads and the duration the
 * stylesheet is allowed to declare are one value in two files, so this reads the other side.
 */
describe('the restoration envelope', () => {
  it('is a duration the style vocabulary carries, compared as a number', () => {
    expect(vocabularyRaw.length, 'css-vocabulary.json read as an empty string').toBeGreaterThan(0);
    const vocabulary = JSON.parse(vocabularyRaw) as { durationsMs: readonly number[] };
    expect(vocabulary.durationsMs.length).toBeGreaterThan(0);
    expect(vocabulary.durationsMs).toContain(RESTORATION_SURGE_MS);
  });
});

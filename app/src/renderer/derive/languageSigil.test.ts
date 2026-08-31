import { describe, expect, it } from 'vitest';
import { LANGUAGE_SIGILS, languageSigil } from './languageSigil.js';

describe('languageSigil', () => {
  it('tags the ten languages §7.3a names', () => {
    expect(languageSigil('Rust')).toBe('.rs');
    expect(languageSigil('TypeScript')).toBe('.ts');
    expect(languageSigil('Python')).toBe('.py');
    expect(languageSigil('C++')).toBe('.cpp');
    expect(languageSigil('C#')).toBe('.cs');
    expect(languageSigil('JavaScript')).toBe('.js');
    expect(languageSigil('Java')).toBe('.java');
    expect(languageSigil('Go')).toBe('.go');
    expect(languageSigil('Shell')).toBe('.sh');
    expect(languageSigil('Lua')).toBe('.lua');
  });

  it('matches case-insensitively, because the projection carries the detector spelling', () => {
    expect(languageSigil('rust')).toBe('.rs');
    expect(languageSigil('TYPESCRIPT')).toBe('.ts');
  });

  // Never render unknown as something. An untagged language drops the field; it never
  // borrows a neighbour's tag and never renders a dash.
  it('returns null for an untagged language and for no language at all', () => {
    expect(languageSigil('Haskell')).toBeNull();
    expect(languageSigil('')).toBeNull();
    expect(languageSigil(null)).toBeNull();
  });

  it('exposes the table so no second copy is written', () => {
    expect(Object.keys(LANGUAGE_SIGILS)).toHaveLength(10);
    expect(Object.values(LANGUAGE_SIGILS).every((s) => s.startsWith('.'))).toBe(true);
  });
});

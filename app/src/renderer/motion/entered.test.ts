import { beforeEach, describe, expect, it } from 'vitest';
import { hasEntered, markEntered, resetEntered } from './entered';

beforeEach(() => {
  resetEntered();
});

describe('entry plays once, per card and per view', () => {
  it('is true the first time and false every time after', () => {
    expect(markEntered('shelf', 'p1')).toBe(true);
    expect(markEntered('shelf', 'p1')).toBe(false);
    expect(hasEntered('shelf', 'p1')).toBe(true);
  });

  it('is per view — the same card entering a second view animates again', () => {
    expect(markEntered('shelf', 'p1')).toBe(true);
    expect(markEntered('reference', 'p1')).toBe(true);
  });

  it('survives a transient state clearing, because it is not render state', () => {
    markEntered('shelf', 'p1');
    // A re-render cannot re-run entry: nothing here is reachable from a component.
    expect(markEntered('shelf', 'p1')).toBe(false);
  });

  it('resets one view without touching the others', () => {
    markEntered('shelf', 'p1');
    markEntered('reference', 'p1');
    resetEntered('shelf');
    expect(hasEntered('shelf', 'p1')).toBe(false);
    expect(hasEntered('reference', 'p1')).toBe(true);
  });
});

import { describe, expect, it } from 'vitest';
import { BODY_SIZES, DISPLAY_SIZES, MONO_SIZES, TYPE_FLOOR_PX, isOnScale, sizesFor } from './type';

describe('the completed scale', () => {
  it('carries the twelve display sizes the last round omitted', () => {
    for (const px of [42, 40, 36, 32, 30, 27, 22, 20, 19, 16, 13.5, 12.5]) {
      expect(DISPLAY_SIZES).toContain(px);
    }
  });

  it('carries §7.7 card copy at body 10 and the whole mono ladder', () => {
    expect(BODY_SIZES).toContain(10);
    expect(MONO_SIZES).toEqual([13, 11.5, 11, 10.5, 10, 9.5, 9, 8.5, 8, 7.5, 7]);
  });

  it('has a floor of 7px and nothing below it', () => {
    expect(TYPE_FLOOR_PX).toBe(7);
    for (const family of ['display', 'body', 'mono'] as const) {
      for (const px of sizesFor(family)) expect(px).toBeGreaterThanOrEqual(7);
    }
  });

  it('does not carry 6.5px, which the prototype breached three times', () => {
    for (const family of ['display', 'body', 'mono'] as const) {
      expect(sizesFor(family)).not.toContain(6.5);
    }
  });

  it('rejects a size that is not a member', () => {
    expect(isOnScale('mono', 7)).toBe(true);
    expect(isOnScale('mono', 6.5)).toBe(false);
    expect(isOnScale('display', 18)).toBe(false);
    expect(isOnScale('display', 17)).toBe(true);
  });
});

import { describe, expect, it } from 'vitest';
import { PALETTE_ROW_STAGGER_MS, paletteRowDelayMs } from './motion.js';

describe('paletteRowDelayMs', () => {
  it('staggers 14 ms per row at full', () => {
    expect(PALETTE_ROW_STAGGER_MS).toBe(14);
    expect(paletteRowDelayMs(0, 'full')).toBe(0);
    expect(paletteRowDelayMs(1, 'full')).toBe(14);
    expect(paletteRowDelayMs(39, 'full')).toBe(546);
  });

  // §11.6: no travelling highlight at reduced, and no animation at all at off.
  it('drops the stagger at reduced and at off', () => {
    for (const i of [0, 1, 39]) {
      expect(paletteRowDelayMs(i, 'reduced')).toBe(0);
      expect(paletteRowDelayMs(i, 'off')).toBe(0);
    }
  });

  it('treats auto as full, because auto is resolved before it reaches here', () => {
    expect(paletteRowDelayMs(2, 'auto')).toBe(28);
  });
});

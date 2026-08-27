import { describe, expect, it } from 'vitest';
import {
  BOOT_FILE_GENERATION,
  DEFAULT_BOOT_FILE,
  parseBootFile,
  serializeBootFile,
} from './bootFile';

describe('parseBootFile', () => {
  it('round-trips through the snake_case on-disk keys', () => {
    const file = {
      generation: BOOT_FILE_GENERATION,
      effectsTier: 'reduced',
      paintFailCount: 1,
      shelfProjection: { tiles: [] },
    } as const;
    const text = serializeBootFile(file);
    expect(text).toContain('"effects_tier"');
    expect(text).toContain('"paint_fail_count"');
    expect(parseBootFile(text)).toEqual(file);
  });

  it('falls back to auto and an empty shelf, never to an error', () => {
    // §11.2a: "a missing, unparseable or wrong-generation file falls back to auto and an
    // empty shelf — never to an error window."
    expect(parseBootFile('}{ not json')).toEqual(DEFAULT_BOOT_FILE);
    expect(parseBootFile('null')).toEqual(DEFAULT_BOOT_FILE);
    expect(parseBootFile('[1,2,3]')).toEqual(DEFAULT_BOOT_FILE);
    expect(parseBootFile('{"generation":99,"effects_tier":"off"}')).toEqual(DEFAULT_BOOT_FILE);
    expect(DEFAULT_BOOT_FILE.effectsTier).toBe('auto');
    expect(DEFAULT_BOOT_FILE.shelfProjection).toBeNull();
  });

  it('repairs individual bad fields without discarding the rest', () => {
    const parsed = parseBootFile(
      `{"generation":${String(BOOT_FILE_GENERATION)},"effects_tier":"sparkly",` +
        `"paint_fail_count":-4,"shelf_projection":{"tiles":[]}}`,
    );
    expect(parsed.effectsTier).toBe('auto');
    expect(parsed.paintFailCount).toBe(0);
    expect(parsed.shelfProjection).toEqual({ tiles: [] });
  });
});

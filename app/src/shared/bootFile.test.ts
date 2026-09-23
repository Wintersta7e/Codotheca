import { describe, expect, it } from 'vitest';
import {
  BOOT_FILE_GENERATION,
  type BootFile,
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
      paintFailForcedAt: 1_700_000_000_000,
      reducedMotionOverride: true,
      shelfProjection: { tiles: [] },
    } as const;
    const text = serializeBootFile(file);
    expect(text).toContain('"effects_tier"');
    expect(text).toContain('"paint_fail_count"');
    expect(text).toContain('"paint_fail_forced_at"');
    expect(text).toContain('"reduced_motion_override"');
    expect(parseBootFile(text)).toEqual(file);
  });

  // Added without a generation bump: a file an earlier build wrote has no such key, and absent
  // means what every earlier file meant — the override was never mirrored, so it reads off.
  it('reads an absent or malformed override as off, and keeps the rest of the file', () => {
    const g = String(BOOT_FILE_GENERATION);
    const read = (json: string): BootFile => parseBootFile(json);
    expect(read(`{"generation":${g},"effects_tier":"full"}`).reducedMotionOverride).toBe(false);
    expect(read(`{"generation":${g},"effects_tier":"full"}`).effectsTier).toBe('full');
    expect(read(`{"generation":${g},"reduced_motion_override":"yes"}`).reducedMotionOverride).toBe(
      false,
    );
    expect(read(`{"generation":${g},"reduced_motion_override":true}`).reducedMotionOverride).toBe(
      true,
    );
    expect(DEFAULT_BOOT_FILE.reducedMotionOverride).toBe(false);
  });

  it('reads no forcing launch rather than a forcing at time zero', () => {
    // §11.3 names the launch that forced the tier off; `0` would name the epoch.
    const at = (json: string): number | null => parseBootFile(json).paintFailForcedAt;
    const g = String(BOOT_FILE_GENERATION);
    expect(at(`{"generation":${g}}`)).toBeNull();
    expect(at(`{"generation":${g},"paint_fail_forced_at":0}`)).toBeNull();
    expect(at(`{"generation":${g},"paint_fail_forced_at":"yesterday"}`)).toBeNull();
    expect(at(`{"generation":${g},"paint_fail_forced_at":5}`)).toBe(5);
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

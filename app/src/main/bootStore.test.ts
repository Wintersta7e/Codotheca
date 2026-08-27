import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { BOOT_FILE_GENERATION, DEFAULT_BOOT_FILE } from '../shared/bootFile';
import { bootFilePath, readBootFile, resolveBootEffectsTier, writeBootFile } from './bootStore';

let dir = '';

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), 'codotheca-boot-'));
});
afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
});

describe('the boot file on disk', () => {
  it('reads back what it wrote', () => {
    const file = {
      generation: BOOT_FILE_GENERATION,
      effectsTier: 'off',
      paintFailCount: 2,
      shelfProjection: null,
    } as const;
    writeBootFile(dir, file);
    expect(readBootFile(dir)).toEqual(file);
  });

  it('creates the directory it was pointed at', () => {
    const nested = join(dir, 'a', 'b');
    writeBootFile(nested, DEFAULT_BOOT_FILE);
    expect(readFileSync(bootFilePath(nested), 'utf8')).toContain('effects_tier');
  });

  it('returns the default when the file is missing or unreadable', () => {
    expect(readBootFile(join(dir, 'nowhere'))).toEqual(DEFAULT_BOOT_FILE);
    writeFileSync(bootFilePath(dir), 'garbage', 'utf8');
    expect(readBootFile(dir)).toEqual(DEFAULT_BOOT_FILE);
  });

  it('leaves no temp file behind', () => {
    writeBootFile(dir, DEFAULT_BOOT_FILE);
    expect(readdirSync(dir)).toEqual(['boot.json']);
  });
});

describe('resolveBootEffectsTier', () => {
  const stored = {
    generation: BOOT_FILE_GENERATION,
    effectsTier: 'full',
    paintFailCount: 0,
    shelfProjection: null,
  } as const;

  it('reads the boot file when nothing overrides it', () => {
    expect(resolveBootEffectsTier({ argv: [], env: {}, stored })).toEqual({
      tier: 'full',
      source: 'boot-file',
    });
  });

  it('lets the environment beat the boot file', () => {
    expect(
      resolveBootEffectsTier({ argv: [], env: { CODOTHECA_EFFECTS_TIER: 'reduced' }, stored }),
    ).toEqual({ tier: 'reduced', source: 'environment' });
  });

  it('lets argv beat the environment', () => {
    expect(
      resolveBootEffectsTier({
        argv: ['app', '--effects-tier=off'],
        env: { CODOTHECA_EFFECTS_TIER: 'full' },
        stored,
      }),
    ).toEqual({ tier: 'off', source: 'argv' });
  });

  it('forces off at two consecutive paint failures', () => {
    // §11.2a: "At 2 it forces off." A user who cannot see the window cannot change a setting
    // inside it, so recovery must not require the GPU.
    const failing = { ...stored, paintFailCount: 2 };
    expect(resolveBootEffectsTier({ argv: [], env: {}, stored: failing })).toEqual({
      tier: 'off',
      source: 'paint-failure',
    });
  });

  it('still honours an explicit override past the force', () => {
    const failing = { ...stored, paintFailCount: 7 };
    expect(
      resolveBootEffectsTier({ argv: ['app', '--effects-tier=full'], env: {}, stored: failing }),
    ).toEqual({ tier: 'full', source: 'argv' });
  });
});

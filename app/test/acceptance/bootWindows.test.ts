import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { expect, test, vi } from 'vitest';

import { bootstrap, clearPaintFailure, type BootstrapDeps } from '../../src/main/bootstrap';
import { PAINT_FAIL_FORCE_OFF_AT, readBootFile } from '../../src/main/bootStore';
import {
  BOOT_FILE_GENERATION,
  BOOT_FILE_NAME,
  DEFAULT_BOOT_FILE,
  parseBootFile,
} from '../../src/shared/bootFile';
import type { BootFile } from '../../src/shared/bootFile';

/**
 * Criterion 57's automated half: `boot.json`, the override order and the paint-failure force.
 *
 * §11.2a's three failure *windows* are prose and geometry and belong to the surfaces plan. The
 * boot file is pure decision logic behind an injected deps object, with no window and no
 * database — which is precisely the property the criterion asserts. **The shell never opens the
 * database**, and the paint-failure recovery must not require the GPU, because a user who cannot
 * see the window cannot change a setting inside it.
 *
 * The names are the join key and are not free text: `acceptance/criteria.json` registers all
 * three verbatim as the `test` field of three `automated` checks, and the acceptance gate fails
 * an automated check whose test did not run. There is deliberately no `describe` around them —
 * vitest's `fullName` prefixes every enclosing suite title, and the registry joins on that.
 *
 * The generation number is imported, never written as a literal: a correct bump would otherwise
 * turn this file red.
 */
function deps(over: Partial<BootstrapDeps> = {}): {
  base: BootstrapDeps;
  written: BootFile[];
} {
  const written: BootFile[] = [];
  const base: BootstrapDeps = {
    argv: [],
    env: {},
    userDataDir: '/does-not-exist',
    registerSchemesAsPrivileged: vi.fn(),
    disableHardwareAcceleration: vi.fn(),
    readBoot: vi.fn(() => DEFAULT_BOOT_FILE),
    writeBoot: vi.fn((_dir: string, file: BootFile) => {
      written.push(file);
    }),
    ...over,
  };
  return { base, written };
}

test('AC-57 boot.json falls back to auto and an empty shelf, never to an error window', () => {
  for (const broken of [
    '',
    '{not json',
    '[]',
    'null',
    JSON.stringify({ generation: BOOT_FILE_GENERATION + 99, effects_tier: 'off' }),
  ]) {
    const parsed = parseBootFile(broken);
    expect(parsed.effectsTier, broken).toBe('auto');
    expect(parsed.shelfProjection, broken).toBeNull();
    expect(parsed.generation, broken).toBe(BOOT_FILE_GENERATION);
    expect(parsed.paintFailCount, broken).toBe(0);
    // "No launch forced it" is not the same claim as "forced at time zero".
    expect(parsed.paintFailForcedAt, broken).toBeNull();
  }

  // Against the production reader, not a fake: a missing directory and an unreadable file are
  // the two cases a first launch actually produces, and the reader is where they are absorbed.
  // `bootstrap` has no try/catch of its own and needs none, which is only true because of this.
  const missing = join(tmpdir(), 'codotheca-boot-does-not-exist', String(Date.now()));
  expect(readBootFile(missing)).toEqual(DEFAULT_BOOT_FILE);

  const dir = mkdtempSync(join(tmpdir(), 'codotheca-boot-'));
  writeFileSync(join(dir, BOOT_FILE_NAME), '{not json', 'utf8');
  expect(readBootFile(dir)).toEqual(DEFAULT_BOOT_FILE);

  // And the whole boot resolves to auto and an empty shelf off that file, with no window and no
  // error path taken.
  const { base } = deps({ userDataDir: dir, readBoot: readBootFile });
  const booted = bootstrap(base);
  expect(booted.tier).toBe('auto');
  expect(booted.stored.shelfProjection).toBeNull();
});

test('AC-57 the effects-tier flag beats the environment variable beats boot.json', () => {
  const stored: BootFile = { ...DEFAULT_BOOT_FILE, effectsTier: 'reduced' };

  const fromFile = deps({ readBoot: vi.fn(() => stored) });
  expect(bootstrap(fromFile.base)).toMatchObject({ tier: 'reduced', source: 'boot-file' });

  const fromEnv = deps({
    readBoot: vi.fn(() => stored),
    env: { CODOTHECA_EFFECTS_TIER: 'full' } as NodeJS.ProcessEnv,
  });
  expect(bootstrap(fromEnv.base)).toMatchObject({ tier: 'full', source: 'environment' });

  const fromArgv = deps({
    readBoot: vi.fn(() => stored),
    env: { CODOTHECA_EFFECTS_TIER: 'full' } as NodeJS.ProcessEnv,
    argv: ['--effects-tier=off'],
  });
  expect(bootstrap(fromArgv.base)).toMatchObject({ tier: 'off', source: 'argv' });

  // Neither override is written back: the file still says what it said.
  for (const run of [fromEnv, fromArgv]) {
    expect(run.written.length).toBeGreaterThan(0);
    for (const file of run.written) expect(file.effectsTier).toBe('reduced');
  }
});

test('AC-57 a second paint failure forces off on the next launch without reading the database', () => {
  const atThreshold: BootFile = { ...DEFAULT_BOOT_FILE, paintFailCount: PAINT_FAIL_FORCE_OFF_AT };
  const forced = deps({ readBoot: vi.fn(() => atThreshold) });
  const result = bootstrap(forced.base);
  expect(result.tier).toBe('off');
  expect(result.source).toBe('paint-failure');
  // The GPU is what may be broken, so the forced path must not require it.
  expect(forced.base.disableHardwareAcceleration).toHaveBeenCalled();

  // One below the threshold is not forced — the counter is a threshold, not a flag.
  const below: BootFile = { ...DEFAULT_BOOT_FILE, paintFailCount: PAINT_FAIL_FORCE_OFF_AT - 1 };
  const notForced = deps({ readBoot: vi.fn(() => below) });
  expect(bootstrap(notForced.base).source).not.toBe('paint-failure');

  // The recovery path does not require the GPU, and does not require the database either.
  const recovering = deps({ readBoot: vi.fn(() => atThreshold) });
  clearPaintFailure({
    userDataDir: recovering.base.userDataDir,
    readBoot: recovering.base.readBoot,
    writeBoot: recovering.base.writeBoot,
  });
  expect(recovering.written.at(-1)?.paintFailCount).toBe(0);

  // The whole decision came from the file. Nothing in BootstrapDeps can open the database.
  expect(Object.keys(recovering.base)).not.toContain('db');
  expect(Object.keys(recovering.base)).not.toContain('index');
});

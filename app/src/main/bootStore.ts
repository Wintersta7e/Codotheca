import { mkdirSync, readFileSync, renameSync, unlinkSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import {
  BOOT_FILE_NAME,
  type BootFile,
  DEFAULT_BOOT_FILE,
  parseBootFile,
  serializeBootFile,
} from '../shared/bootFile';
import {
  EFFECTS_TIER_ENV_VAR,
  type EffectsTier,
  effectsTierFromArgv,
  parseEffectsTier,
} from '../shared/effectsTier';

export type EffectsTierSource = 'argv' | 'environment' | 'paint-failure' | 'boot-file';

/** §11.2a: "At 2 it forces `off`." */
export const PAINT_FAIL_FORCE_OFF_AT = 2;

export function bootFilePath(userDataDir: string): string {
  return join(userDataDir, BOOT_FILE_NAME);
}

export function readBootFile(userDataDir: string): BootFile {
  try {
    return parseBootFile(readFileSync(bootFilePath(userDataDir), 'utf8'));
  } catch {
    return DEFAULT_BOOT_FILE;
  }
}

/** Temp-and-rename, the way §1.12 writes its sidecar: a torn write is never observable. */
export function writeBootFile(userDataDir: string, file: BootFile): void {
  mkdirSync(userDataDir, { recursive: true });
  const target = bootFilePath(userDataDir);
  const temporary = `${target}.${String(process.pid)}.tmp`;
  try {
    writeFileSync(temporary, serializeBootFile(file), 'utf8');
    renameSync(temporary, target);
  } catch (error: unknown) {
    try {
      unlinkSync(temporary);
    } catch {
      // The temp file was never created. Nothing to clean up.
    }
    throw error;
  }
}

/**
 * §11.2a's override chain: `--effects-tier=` beats `CODOTHECA_EFFECTS_TIER`, and both beat
 * `boot.json`. The forced `off` sits between the environment and the file — it must survive a
 * stored `full`, because a user who cannot see the window cannot change a setting inside it,
 * but it must not survive an operator deliberately asking for a tier on the command line.
 */
export function resolveBootEffectsTier(input: {
  readonly argv: readonly string[];
  readonly env: NodeJS.ProcessEnv;
  readonly stored: BootFile;
}): { readonly tier: EffectsTier; readonly source: EffectsTierSource } {
  const fromArgv = effectsTierFromArgv(input.argv);
  if (fromArgv !== null) {
    return { tier: fromArgv, source: 'argv' };
  }
  const fromEnvironment = parseEffectsTier(input.env[EFFECTS_TIER_ENV_VAR]);
  if (fromEnvironment !== null) {
    return { tier: fromEnvironment, source: 'environment' };
  }
  if (input.stored.paintFailCount >= PAINT_FAIL_FORCE_OFF_AT) {
    return { tier: 'off', source: 'paint-failure' };
  }
  return { tier: input.stored.effectsTier, source: 'boot-file' };
}

import { type EffectsTier, parseEffectsTier } from './effectsTier';

/**
 * `boot.json` — shell-owned, in the app data dir (§11.2a).
 *
 * The database is authoritative and this file is a mirror: on join the core's value wins and
 * the file is rewritten if it differs, and nothing reads it after the join. It exists because
 * §1.10 gives the core the only database connection and the core joins *after* first paint,
 * which would otherwise make the one setting whose premise is a broken GPU unreachable at the
 * moment it is required.
 *
 * On-disk keys are snake_case, matching the `app_meta` column names they mirror.
 */
export const BOOT_FILE_NAME = 'boot.json';

/** Bumped when the record's shape changes. A wrong generation is discarded, never migrated. */
export const BOOT_FILE_GENERATION = 1;

export interface BootFile {
  readonly generation: number;
  readonly effectsTier: EffectsTier;
  /**
   * Incremented before the window is created, cleared on the first composited frame. At
   * `PAINT_FAIL_FORCE_OFF_AT` it forces `off`.
   */
  readonly paintFailCount: number;
  /**
   * The last shelf projection (§8.2) the UI lane paints from. Typed `unknown` here on
   * purpose: plan 13 owns the projection's shape and validates this field. Anything read out
   * of it before then is untrusted disk content.
   */
  readonly shelfProjection: unknown;
}

export const DEFAULT_BOOT_FILE: BootFile = {
  generation: BOOT_FILE_GENERATION,
  effectsTier: 'auto',
  paintFailCount: 0,
  shelfProjection: null,
};

/** Never throws. Anything it cannot understand becomes {@link DEFAULT_BOOT_FILE}. */
export function parseBootFile(text: string): BootFile {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return DEFAULT_BOOT_FILE;
  }
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    return DEFAULT_BOOT_FILE;
  }
  const record = raw as Record<string, unknown>;
  if (record['generation'] !== BOOT_FILE_GENERATION) {
    return DEFAULT_BOOT_FILE;
  }
  const tierValue = record['effects_tier'];
  const countValue = record['paint_fail_count'];
  return {
    generation: BOOT_FILE_GENERATION,
    effectsTier: parseEffectsTier(typeof tierValue === 'string' ? tierValue : undefined) ?? 'auto',
    paintFailCount:
      typeof countValue === 'number' && Number.isInteger(countValue) && countValue >= 0
        ? countValue
        : 0,
    shelfProjection: 'shelf_projection' in record ? record['shelf_projection'] : null,
  };
}

export function serializeBootFile(file: BootFile): string {
  return `${JSON.stringify(
    {
      generation: file.generation,
      effects_tier: file.effectsTier,
      paint_fail_count: file.paintFailCount,
      shelf_projection: file.shelfProjection,
    },
    null,
    2,
  )}\n`;
}

// The shell half of §11.2a's report. It never opens the database — §1.10 gives the core the
// only connection, and all three of these windows exist precisely because the database will
// not open. The core leaves one small JSON file beside it and exits; this reads it.
import { readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
// The shape moved to `src/shared` so the renderer's failure windows can name it: this file
// opens with `node:fs`, and `tsconfig.web.json` carries no Node types, so importing the types
// from here fails the renderer's typecheck with `Cannot find module 'node:fs'`. Re-exported so
// every existing caller of this module is unchanged.
import type { StartupFailure } from '../shared/startupFailure';

export type { LedgerCounts, StartupFailure } from '../shared/startupFailure';

export const STARTUP_FAILURE_FILE = 'startup-failure.json';
export const EXIT_INDEX_FATAL = 4;

const KINDS = ['schema_from_future', 'migration_failed', 'corrupt_index'] as const;

/** Never throws. A missing, truncated or unrecognised report is no failure at all. */
export function readStartupFailure(dataDir: string): StartupFailure | null {
  let text: string;
  try {
    text = readFileSync(join(dataDir, STARTUP_FAILURE_FILE), 'utf8');
  } catch {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return null;
  }
  if (typeof parsed !== 'object' || parsed === null) return null;
  const kind = (parsed as { kind?: unknown }).kind;
  if (typeof kind !== 'string' || !KINDS.includes(kind as (typeof KINDS)[number])) return null;
  return parsed as StartupFailure;
}

export function clearStartupFailure(dataDir: string): void {
  rmSync(join(dataDir, STARTUP_FAILURE_FILE), { force: true });
}

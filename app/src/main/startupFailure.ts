// The shell half of §11.2a's report. It never opens the database — §1.10 gives the core the
// only connection, and all three of these windows exist precisely because the database will
// not open. The core leaves one small JSON file beside it and exits; this reads it.
import { readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';

export const STARTUP_FAILURE_FILE = 'startup-failure.json';
export const EXIT_INDEX_FATAL = 4;

/** One block of §11.2a's ledger. */
export interface LedgerCounts {
  readonly projects: number;
  readonly notes: number;
  readonly sessions: number;
  readonly collections: number;
  readonly roots: number;
  readonly xpEvents: number;
  readonly launchTargets: number;
}

export type StartupFailure =
  | { readonly kind: 'schema_from_future'; readonly onDisk: number; readonly supported: number }
  | {
      readonly kind: 'migration_failed';
      readonly version: number;
      readonly name: string;
      readonly restoredTo: number;
      readonly restoredAt: number;
    }
  | {
      readonly kind: 'corrupt_index';
      readonly quarantinedAt: number;
      readonly gapStartedAt: number | null;
      readonly gapCountsRecoverable: boolean;
      /**
       * `null` until a rebuild has run. The window draws no figure at all for a null block —
       * `0 projects restorable` would be a claim about what was lost, on the one screen where
       * unknown-as-zero does the most damage.
       */
      readonly reDerivable: LedgerCounts | null;
      readonly restorable: LedgerCounts | null;
    };

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

/**
 * §11.2a's report as pure data, in `shared` so both processes can name it.
 *
 * The reader lives in `src/main` because reading it is a filesystem act; the *shape* is not.
 * `app/tsconfig.web.json` includes `src/renderer`, `src/shared` and `src/generated` and nothing
 * else, and `src/main/startupFailure.ts` opens with `node:fs` — so a renderer module importing
 * these types from there fails `tsc -p tsconfig.web.json` with `Cannot find module 'node:fs'`,
 * which reads as a missing dependency and is a project-boundary crossing. Same move, and the
 * same reason, as `EffectsTierSource`.
 */

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
      readonly sidecar: SidecarReport;
      /** Why the last REBUILD did not complete; `null` when none has been tried. */
      readonly rebuildFailed: string | null;
      /** §48.7.2: what was decided after the sidecar's write cannot be counted before a rebuild. */
      readonly gapCountsRecoverable: false;
    };

/**
 * The `corrupt_index` report's fields besides its `kind`, as the core serialises them. The shell
 * reads a report whose keys are exactly these and refuses any other.
 */
export const CORRUPT_INDEX_FIELDS = ['sidecar', 'rebuildFailed', 'gapCountsRecoverable'] as const;

/** `SidecarReport`'s fields, as the core serialises them. */
export const SIDECAR_REPORT_FIELDS = [
  'state',
  'writtenAt',
  'generation',
  'counts',
  'reason',
] as const;

/** What the sidecar beside a corrupt index is, read before any rebuild (§48.7.1 step 2). */
export const SIDECAR_STATES = ['present', 'absent', 'unreadable', 'newer'] as const;

/**
 * The sidecar a REBUILD would restore from. `writtenAt`, `generation` and `counts` are known only
 * for a `present` sidecar and `null` otherwise; `reason` says why one is `unreadable` or `newer`.
 */
export interface SidecarReport {
  readonly state: (typeof SIDECAR_STATES)[number];
  readonly writtenAt: number | null;
  readonly generation: number | null;
  readonly counts: Readonly<Record<string, number>> | null;
  readonly reason: string | null;
}

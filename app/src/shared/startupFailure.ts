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

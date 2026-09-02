/**
 * The core lane's state as pure data, in `shared` so both processes can name it.
 *
 * It is sent verbatim over `IPC_CORE_STATUS`, so the renderer has to be able to read it — and
 * `app/tsconfig.web.json` includes `src/renderer`, `src/shared` and `src/generated` and nothing
 * else. `src/main/core/supervisor.ts` opens with `node:child_process`, so a renderer module
 * naming these types from there fails `tsc -p tsconfig.web.json` with a missing-module error
 * that reads as a broken dependency and is a project-boundary crossing. Same move, and the same
 * reason, as `StartupFailure` and `EffectsTierSource`.
 */

import type { StartupFailure } from './startupFailure';

export type CoreFailureReason = 'spawn' | 'protocol_version' | 'crash_loop';

/**
 * `logPath` rides on `failed` because that is the one state where the shell has a path the user
 * needs and the renderer has none of its own (§2.4 — the renderer originates no path).
 */
export type CoreStatus =
  | { kind: 'starting' }
  | { kind: 'ready'; epoch: number; coreVersion: string; protocolVersion: number; pid: number }
  | { kind: 'restarting'; epoch: number; delayMs: number }
  | {
      kind: 'failed';
      reason: CoreFailureReason;
      detail: string;
      logPath: string;
      /**
       * §11.2a's report, re-read at the moment the lane is declared failed.
       *
       * Optional because the supervisor does not read the filesystem: the shell attaches it on
       * the way to the renderer. It cannot ride on the window's argv instead — the core writes
       * the file *after* the window is created, so a value read at window creation is `null`
       * for the first occurrence of a fault and stale after a repair, in both directions.
       */
      startupFailure?: StartupFailure | null;
    };

const KINDS = ['starting', 'ready', 'restarting', 'failed'] as const;

/**
 * Shape, not value. The shell is the only producer on this channel; what this guards against is
 * a frame from an older shell, not a hostile one.
 */
export function isCoreStatus(value: unknown): value is CoreStatus {
  if (typeof value !== 'object' || value === null) return false;
  const kind = (value as { kind?: unknown }).kind;
  return typeof kind === 'string' && (KINDS as readonly string[]).includes(kind);
}

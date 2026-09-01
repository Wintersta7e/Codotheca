// §11.2's core lane joins after first paint; these two steps run there.
//
// §11.5's premise, stated once: a Squirrel-installed editor lives in a versioned directory and
// moves on every update, so a stored path goes stale routinely rather than exceptionally.
// `targets.verify` runs at startup for that reason, and its result feeds both the drawer's
// rows and §8.0's priority-4 notice.
import type { CommandName, Settings, TargetVerification } from '../generated/protocol';
import type { JoinStep } from './startup';

type Request = (name: CommandName, args: unknown) => Promise<unknown>;

/** `{targetId: null}` is §4bis.5's all-rows sweep; §11.5's per-target window passes an id. */
export function verifyTargetsStep(
  request: Request,
  onResult: (rows: readonly TargetVerification[]) => void,
): JoinStep {
  return {
    name: 'targets.verify',
    run: async () => {
      const rows = (await request('targets.verify', { targetId: null })) as TargetVerification[];
      onResult(rows);
    },
  };
}

/**
 * `unverified` means *not checked*, and `not_executable` is a fact about a file that is still
 * there. Neither is *gone*; only `missing` is stale, and §11.5 draws a different window for
 * each — folding them would send the user looking for a file that never moved.
 */
export function staleTargets(rows: readonly TargetVerification[]): readonly TargetVerification[] {
  return rows.filter((r) => r.verifyState === 'missing');
}

export function logLevelStep(
  request: Request,
  log: { setLevel(level: Settings['logLevel']): void },
): JoinStep {
  return {
    name: 'log.level',
    run: async () => {
      const settings = (await request('settings.get', {})) as Settings;
      log.setLevel(settings.logLevel);
    },
  };
}

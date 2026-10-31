// §11.2's core lane joins after first paint; these two steps run there.
//
// §11.5's premise, stated once: a Squirrel-installed editor lives in a versioned directory and
// moves on every update, so a stored path goes stale routinely rather than exceptionally.
// `targets.verify` runs at startup for that reason, and its result feeds both the drawer's
// rows and §8.0's priority-4 notice — the notice through `staleTargetsRelay`.
import type { CommandName, Settings, TargetVerification } from '../generated/protocol';
import { IPC_STALE_TARGETS, IPC_STALE_TARGETS_NOW } from '../shared/channels';
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

/**
 * §8.0's priority-4 notice, fed from the startup sweep. `staleTargets` is its one predicate: the
 * window is handed the stale rows and never filters again. `send` pushes them to a window that
 * is listening; `now` answers one that mounted after the push.
 */
export function staleTargetsRelay(send: (rows: readonly TargetVerification[]) => void): {
  readonly onResult: (rows: readonly TargetVerification[]) => void;
  readonly now: () => readonly TargetVerification[] | null;
} {
  let last: readonly TargetVerification[] | null = null;
  return {
    onResult: (rows) => {
      const stale = staleTargets(rows);
      last = stale;
      send(stale);
    },
    now: () => last,
  };
}

/**
 * The relay on its two channels: the push on `IPC_STALE_TARGETS`, the late-mount answer on
 * `IPC_STALE_TARGETS_NOW` — `null` until the startup sweep lands, never an empty list that would
 * claim a check that has not run.
 */
export function registerStaleTargets(deps: {
  readonly handle: (channel: string, fn: () => unknown) => void;
  readonly send: (channel: string, rows: readonly TargetVerification[]) => void;
}): ReturnType<typeof staleTargetsRelay> {
  const relay = staleTargetsRelay((rows) => {
    deps.send(IPC_STALE_TARGETS, rows);
  });
  deps.handle(IPC_STALE_TARGETS_NOW, () => relay.now());
  return relay;
}

/**
 * §8.6's chord is stored in `settings` and registered by this process, so something has to read
 * the one and apply it to the other. Without this a stored chord silently does nothing after a
 * restart, and the drawer's row would say `NOT SET` about a setting that is plainly set.
 */
export function residentShortcutStep(
  request: Request,
  apply: (chord: string | null) => unknown,
): JoinStep {
  return {
    name: 'shortcut.bind',
    run: async () => {
      const settings = (await request('settings.get', {})) as Settings;
      apply(settings.residentShortcut);
    },
  };
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

/**
 * §11.2a: on join the core's tier wins and `boot.json` is rewritten if it differs. Without this
 * the file keeps what it last held — `auto` on every install — and the tier a user stored never
 * reaches the first frame of a later launch.
 */
export function bootMirrorStep(request: Request, mirror: (settings: Settings) => void): JoinStep {
  return {
    name: 'boot.mirror',
    run: async () => {
      mirror((await request('settings.get', {})) as Settings);
    },
  };
}

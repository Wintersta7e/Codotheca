/**
 * §11.2's startup: two lanes from the advisory lock onward, joined later.
 *
 * The UI lane paints from the last snapshot and is interactive before the handshake
 * completes. Putting the core spawn and the git version check on the critical path is what
 * made the cold-launch criterion unreachable in v2.
 */
import type { RollingLog } from './core/log';
import type { CoreFailureReason, CoreStatus } from './core/supervisor';

export interface StartupSupervisor {
  readonly status: CoreStatus;
  onStatus(fn: (s: CoreStatus) => void): void;
  start(): void;
}

/** Ordered work that runs once the handshake completes. The order encodes the dependency. */
export interface JoinStep {
  name: string;
  run: () => Promise<void>;
}

export type JoinResult =
  | { kind: 'joined' }
  | { kind: 'core_failed'; reason: CoreFailureReason }
  | { kind: 'step_failed'; step: string; detail: string };

export type StartupOutcome =
  | { kind: 'second_instance' }
  | { kind: 'blocked' }
  | { kind: 'running'; interactiveAfterMs: number; joined: Promise<JoinResult> };

export interface StartupDeps {
  acquireInstanceLock(): boolean;
  focusExistingWindow(): void;
  dataDir: string;
  supervisor: StartupSupervisor;
  log: RollingLog;
  now(): number;
  /** Restore geometry, paint from the last snapshot, become interactive. */
  paintUiLane(): Promise<void>;
  /** Polls the core's advisory lock. `aborted` when the user gives up or forces. */
  awaitFreeLock(): Promise<'free' | 'aborted'>;
  joinSteps: JoinStep[];
}

async function runJoin(deps: StartupDeps): Promise<JoinResult> {
  const ready = await new Promise<CoreStatus>((resolve) => {
    if (deps.supervisor.status.kind === 'ready' || deps.supervisor.status.kind === 'failed') {
      resolve(deps.supervisor.status);
      return;
    }
    deps.supervisor.onStatus((s) => {
      if (s.kind === 'ready' || s.kind === 'failed') resolve(s);
    });
  });
  if (ready.kind === 'failed') {
    deps.log.write('error', 'shell', `core lane failed: ${ready.reason}`);
    return { kind: 'core_failed', reason: ready.reason };
  }
  for (const step of deps.joinSteps) {
    try {
      await step.run();
    } catch (e: unknown) {
      deps.log.write('error', 'shell', `join step ${step.name} failed: ${String(e)}`);
      return { kind: 'step_failed', step: step.name, detail: String(e) };
    }
  }
  return { kind: 'joined' };
}

export async function runStartup(deps: StartupDeps): Promise<StartupOutcome> {
  if (!deps.acquireInstanceLock()) {
    deps.focusExistingWindow();
    return { kind: 'second_instance' };
  }
  if ((await deps.awaitFreeLock()) === 'aborted') {
    return { kind: 'blocked' };
  }
  const started = deps.now();
  // The core lane is subscribed before it is started, so a handshake that completes during
  // the paint is not missed.
  const joined = runJoin(deps);
  deps.supervisor.start();
  await deps.paintUiLane();
  return { kind: 'running', interactiveAfterMs: deps.now() - started, joined };
}

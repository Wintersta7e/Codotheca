/**
 * Process lifecycle: spawn, handshake, stderr drain, restart policy.
 *
 * One automatic restart with a 2 s backoff; a second crash inside 60 s stops retrying and
 * reports the log path, which is what the failure surface shows.
 */
import { PROTOCOL_VERSION } from '../../generated/protocol';
import type { CoreFailureReason, CoreStatus } from '../../shared/coreStatus';
import { FrameDecoder, encodeFrame } from './frame';
import { type RollingLog, drainStderr } from './log';
import type { CoreChild, SpawnCore } from './spawn';
import { type Inbound, type Outbound, parseOutbound } from './wire';

export const RESTART_BACKOFF_MS = 2_000;
export const CRASH_LOOP_WINDOW_MS = 60_000;
/** Reserved for the shell's own `app.hello_ack`; client-issued ids start at 1. */
export const HELLO_REQUEST_ID = 0;

// The shape moved to `src/shared` so the renderer can name what arrives on `IPC_CORE_STATUS`:
// this file opens with the shell's own modules, which `tsconfig.web.json` cannot see.
// Re-exported so every existing caller of this module is unchanged.
export type { CoreFailureReason, CoreStatus } from '../../shared/coreStatus';

export interface SupervisorDeps {
  binaryPath: string;
  dataDir: string;
  log: RollingLog;
  spawn: SpawnCore;
  now: () => number;
  schedule: (fn: () => void, ms: number) => void;
}

export class CoreSupervisor {
  private readonly deps: SupervisorDeps;
  private child: CoreChild | null = null;
  private epoch = 0;
  private lastCrashAt: number | null = null;
  private state: CoreStatus = { kind: 'starting' };
  private stopping = false;
  private readonly statusListeners: ((s: CoreStatus) => void)[] = [];
  private readonly frameListeners: ((f: Outbound) => void)[] = [];
  private readonly epochEndListeners: ((epoch: number) => void)[] = [];

  constructor(deps: SupervisorDeps) {
    this.deps = deps;
  }

  get status(): CoreStatus {
    return this.state;
  }

  get currentEpoch(): number {
    return this.epoch;
  }

  onStatus(fn: (s: CoreStatus) => void): void {
    this.statusListeners.push(fn);
  }

  onFrame(fn: (f: Outbound) => void): void {
    this.frameListeners.push(fn);
  }

  /** Fires the instant the core dies, before any restart. Every pending request dies here. */
  onEpochEnd(fn: (epoch: number) => void): void {
    this.epochEndListeners.push(fn);
  }

  start(): void {
    this.epoch += 1;
    this.setStatus({ kind: 'starting' });
    let child: CoreChild;
    try {
      child = this.deps.spawn({
        binaryPath: this.deps.binaryPath,
        dataDir: this.deps.dataDir,
        epoch: this.epoch,
        parentPid: process.pid,
      });
    } catch (e: unknown) {
      this.fail('spawn', String(e));
      return;
    }
    this.child = child;
    drainStderr(child.stderr, this.deps.log);

    const decoder = new FrameDecoder();
    child.stdout.on('data', (chunk: Buffer) => {
      let bodies: string[];
      try {
        bodies = decoder.push(chunk);
      } catch (e: unknown) {
        this.deps.log.write('error', 'shell', `protocol kill: ${String(e)}`);
        child.kill();
        return;
      }
      for (const body of bodies) {
        const frame = parseOutbound(body);
        if (frame === null) {
          this.deps.log.write('error', 'shell', 'protocol kill: unparseable frame');
          child.kill();
          return;
        }
        this.dispatch(frame);
      }
    });

    child.onError((err) => {
      this.fail('spawn', err.message);
    });
    child.onExit((code, signal) => {
      this.onDeath(code, signal);
    });
  }

  send(frame: Inbound): boolean {
    const child = this.child;
    if (child === null || this.state.kind === 'failed') return false;
    return child.stdin.write(encodeFrame(JSON.stringify(frame)));
  }

  /** Orderly shutdown: closing stdin is the core's exit signal. */
  stop(): void {
    this.stopping = true;
    this.child?.stdin.end();
  }

  private dispatch(frame: Outbound): void {
    if (frame.t === 'hello') {
      if (frame.protocol_version !== PROTOCOL_VERSION) {
        this.fail(
          'protocol_version',
          `core speaks protocol ${String(frame.protocol_version)}, this build speaks ${String(PROTOCOL_VERSION)}`,
        );
        this.child?.kill();
        return;
      }
      this.send({
        t: 'request',
        id: HELLO_REQUEST_ID,
        command: 'app.hello_ack',
        args: { protocolVersion: PROTOCOL_VERSION },
      });
      this.setStatus({
        kind: 'ready',
        epoch: frame.epoch,
        coreVersion: frame.core_version,
        protocolVersion: frame.protocol_version,
        pid: frame.pid,
      });
      return;
    }
    // Request ids and event sequences are scoped to the epoch; a frame from a dead one is
    // dropped here so no listener has to think about it.
    if (frame.epoch !== this.epoch) return;
    if (frame.t === 'response' && frame.id === HELLO_REQUEST_ID) return;
    for (const fn of this.frameListeners) fn(frame);
  }

  private onDeath(code: number | null, signal: NodeJS.Signals | null): void {
    const dead = this.epoch;
    this.child = null;
    for (const fn of this.epochEndListeners) fn(dead);
    if (this.stopping || this.state.kind === 'failed') return;
    this.deps.log.write(
      'error',
      'shell',
      `core exited code=${String(code)} signal=${String(signal)}`,
    );
    const at = this.deps.now();
    const previous = this.lastCrashAt;
    this.lastCrashAt = at;
    if (previous !== null && at - previous < CRASH_LOOP_WINDOW_MS) {
      this.fail('crash_loop', `two crashes within ${String(CRASH_LOOP_WINDOW_MS)} ms`);
      return;
    }
    this.setStatus({ kind: 'restarting', epoch: dead, delayMs: RESTART_BACKOFF_MS });
    this.deps.schedule(() => {
      this.start();
    }, RESTART_BACKOFF_MS);
  }

  private fail(reason: CoreFailureReason, detail: string): void {
    this.deps.log.write('error', 'shell', `core failed (${reason}): ${detail}`);
    this.setStatus({ kind: 'failed', reason, detail, logPath: this.deps.log.path });
  }

  private setStatus(next: CoreStatus): void {
    this.state = next;
    for (const fn of this.statusListeners) fn(next);
  }
}

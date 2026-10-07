import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { PassThrough } from 'node:stream';
import { describe, expect, it } from 'vitest';
import { PROTOCOL_VERSION } from '../../generated/protocol';
import { EXIT_INDEX_FATAL } from '../startupFailure';
import { encodeFrame } from './frame';
import { type RollingLog, openRollingLog } from './log';
import type { CoreArgv, CoreChild } from './spawn';
import {
  CRASH_LOOP_WINDOW_MS,
  CoreSupervisor,
  RESTART_BACKOFF_MS,
  type CoreStatus,
} from './supervisor';

interface Harness {
  readonly sup: CoreSupervisor;
  readonly statuses: CoreStatus[];
  readonly children: FakeChild[];
  /** The argv of every spawn, in order. */
  readonly argvs: CoreArgv[];
  readonly timers: { fn: () => void; ms: number }[];
  clock: number;
  cleanup(): void;
}

class FakeChild implements CoreChild {
  readonly pid = 1234;
  readonly stdin = new PassThrough();
  readonly stdout = new PassThrough();
  readonly stderr = new PassThrough();
  killed = false;
  private exitCb: ((c: number | null, s: NodeJS.Signals | null) => void) | null = null;
  private errorCb: ((err: Error) => void) | null = null;
  kill(): void {
    this.killed = true;
  }
  onExit(cb: (c: number | null, s: NodeJS.Signals | null) => void): void {
    this.exitCb = cb;
  }
  onError(cb: (err: Error) => void): void {
    this.errorCb = cb;
  }
  fault(err: Error): void {
    this.errorCb?.(err);
  }
  greet(protocolVersion: number): void {
    this.stdout.write(
      encodeFrame(
        JSON.stringify({
          t: 'hello',
          protocol_version: protocolVersion,
          core_version: '0.0.0',
          epoch: 1,
          pid: 1234,
        }),
      ),
    );
  }
  die(code = 101): void {
    this.exitCb?.(code, null);
  }
}

function harness(): Harness {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'codotheca-sup-'));
  const log: RollingLog = openRollingLog({ dir, maxBytes: 1_000_000, keep: 1, level: 'debug' });
  const children: FakeChild[] = [];
  const argvs: CoreArgv[] = [];
  const timers: { fn: () => void; ms: number }[] = [];
  const state = { clock: 0 };
  const sup = new CoreSupervisor({
    binaryPath: '/nowhere/codotheca-core',
    workerPath: null,
    dataDir: dir,
    log,
    spawn: (argv) => {
      argvs.push(argv);
      const c = new FakeChild();
      children.push(c);
      return c;
    },
    now: () => state.clock,
    schedule: (fn, ms) => {
      timers.push({ fn, ms });
    },
  });
  const statuses: CoreStatus[] = [];
  sup.onStatus((s) => statuses.push(s));
  return {
    sup,
    statuses,
    children,
    argvs,
    timers,
    get clock(): number {
      return state.clock;
    },
    set clock(v: number) {
      state.clock = v;
    },
    cleanup: () => {
      log.close();
      fs.rmSync(dir, { recursive: true, force: true });
    },
  };
}

const settle = (): Promise<void> => new Promise((r) => setImmediate(r));

describe('core supervisor', () => {
  it('acks a matching hello and reports ready', async () => {
    const h = harness();
    h.sup.start();
    h.children[0]?.greet(PROTOCOL_VERSION);
    await settle();
    expect(h.statuses.at(-1)?.kind).toBe('ready');
    h.cleanup();
  });

  it('fails a protocol version mismatch without restarting', async () => {
    const h = harness();
    h.sup.start();
    h.children[0]?.greet(PROTOCOL_VERSION + 1);
    await settle();
    const last = h.statuses.at(-1);
    expect(last?.kind).toBe('failed');
    expect(last?.kind === 'failed' ? last.reason : null).toBe('protocol_version');
    expect(h.children[0]?.killed).toBe(true);
    h.cleanup();
  });

  it('restarts once after the backoff', async () => {
    const h = harness();
    h.sup.start();
    h.children[0]?.greet(PROTOCOL_VERSION);
    await settle();
    h.children[0]?.die();
    await settle();
    expect(h.statuses.at(-1)?.kind).toBe('restarting');
    expect(h.timers[0]?.ms).toBe(RESTART_BACKOFF_MS);
    h.timers[0]?.fn();
    expect(h.children.length).toBe(2);
    h.cleanup();
  });

  it('stops retrying on a second crash inside the window and names the log', async () => {
    const h = harness();
    h.sup.start();
    h.children[0]?.greet(PROTOCOL_VERSION);
    await settle();
    h.children[0]?.die();
    await settle();
    h.timers[0]?.fn();
    h.clock = CRASH_LOOP_WINDOW_MS - 1;
    h.children[1]?.die();
    await settle();
    const last = h.statuses.at(-1);
    expect(last?.kind).toBe('failed');
    expect(last?.kind === 'failed' ? last.reason : null).toBe('crash_loop');
    expect(last?.kind === 'failed' && last.logPath.endsWith('codotheca.log')).toBe(true);
    expect(h.timers.length).toBe(1);
    h.cleanup();
  });

  // §48.7.1 step 2: the core exits EXIT_INDEX_FATAL having written its report. Restarting it
  // only writes the same report again, and the restart's `starting` is what the window shows.
  it('fails once with index_fatal on exit 4 and never restarts', async () => {
    const h = harness();
    h.sup.start();
    h.children[0]?.greet(PROTOCOL_VERSION);
    await settle();
    h.children[0]?.die(EXIT_INDEX_FATAL);
    await settle();
    const reasons = h.statuses.flatMap((s) => (s.kind === 'failed' ? [s.reason] : []));
    expect(reasons).toEqual(['index_fatal']);
    expect(h.statuses.map((s) => s.kind)).not.toContain('restarting');
    expect(h.timers).toEqual([]);
    expect(h.children.length).toBe(1);
    h.cleanup();
  });

  // Measured against the release core: the ack is written to a core that has already exited,
  // so the stdin EPIPE reaches the error sink before the exit does, and failing on it reported
  // a fatal index as a spawn failure. The exit code is what says why the core stopped.
  it('leaves a stdin EPIPE to the exit that follows it', async () => {
    const h = harness();
    h.sup.start();
    h.children[0]?.greet(PROTOCOL_VERSION);
    await settle();
    h.children[0]?.fault(Object.assign(new Error('write EPIPE'), { code: 'EPIPE' }));
    h.children[0]?.die(EXIT_INDEX_FATAL);
    await settle();
    const reasons = h.statuses.flatMap((s) => (s.kind === 'failed' ? [s.reason] : []));
    expect(reasons).toEqual(['index_fatal']);
    h.cleanup();
  });

  it('still fails spawn on an error that is not a closed pipe', () => {
    const h = harness();
    h.sup.start();
    h.children[0]?.fault(Object.assign(new Error('spawn ENOENT'), { code: 'ENOENT' }));
    const last = h.statuses.at(-1);
    expect(last?.kind === 'failed' ? last.reason : null).toBe('spawn');
    h.cleanup();
  });

  // §48.7.1 step 3: REBUILD is a startup mode. The flag says "this index is corrupt, rebuild
  // it", which only the user's press asserts, so a later automatic restart spawns without it.
  it('rebuild() spawns once with the flag, and the next restart does not', async () => {
    const h = harness();
    h.sup.start();
    h.children[0]?.die(EXIT_INDEX_FATAL);
    await settle();
    expect(h.sup.rebuild()).toBe(true);
    // A second press while the rebuild starts is not a second rebuild.
    expect(h.sup.rebuild()).toBe(false);
    expect(h.children.length).toBe(2);
    expect(h.statuses.at(-1)?.kind).toBe('starting');
    h.children[1]?.greet(PROTOCOL_VERSION);
    await settle();
    expect(h.statuses.at(-1)?.kind).toBe('ready');
    h.children[1]?.die();
    await settle();
    h.timers[0]?.fn();
    expect(h.argvs.map((a) => a.rebuild)).toEqual([false, true, false]);
    h.cleanup();
  });

  it('refuses a rebuild unless the lane has failed', async () => {
    const h = harness();
    h.sup.start();
    expect(h.sup.rebuild()).toBe(false);
    h.children[0]?.greet(PROTOCOL_VERSION);
    await settle();
    expect(h.sup.rebuild()).toBe(false);
    expect(h.children.length).toBe(1);
    h.cleanup();
  });

  // The press is the user's act, not a crash: a crash before it must not count against the core
  // it starts, or that core's first crash inside the window would read as a loop and end it.
  it('rebuild() resets the crash window', async () => {
    const h = harness();
    h.sup.start();
    h.children[0]?.die();
    await settle();
    h.timers[0]?.fn();
    h.children[1]?.die(EXIT_INDEX_FATAL);
    await settle();
    expect(h.sup.rebuild()).toBe(true);
    h.clock = 1_000;
    h.children[2]?.die();
    await settle();
    expect(h.statuses.at(-1)?.kind).toBe('restarting');
    h.cleanup();
  });
});

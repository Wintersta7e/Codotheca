import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { PassThrough } from 'node:stream';
import { describe, expect, it } from 'vitest';
import { PROTOCOL_VERSION } from '../../generated/protocol';
import { encodeFrame } from './frame';
import { type RollingLog, openRollingLog } from './log';
import type { CoreChild } from './spawn';
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
  kill(): void {
    this.killed = true;
  }
  onExit(cb: (c: number | null, s: NodeJS.Signals | null) => void): void {
    this.exitCb = cb;
  }
  onError(): void {
    /* unused in these tests */
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
  die(): void {
    this.exitCb?.(101, null);
  }
}

function harness(): Harness {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'codotheca-sup-'));
  const log: RollingLog = openRollingLog({ dir, maxBytes: 1_000_000, keep: 1, level: 'debug' });
  const children: FakeChild[] = [];
  const timers: { fn: () => void; ms: number }[] = [];
  const state = { clock: 0 };
  const sup = new CoreSupervisor({
    binaryPath: '/nowhere/codotheca-core',
    dataDir: dir,
    log,
    spawn: () => {
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
});

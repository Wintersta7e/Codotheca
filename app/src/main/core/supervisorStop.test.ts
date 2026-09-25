import { describe, expect, it } from 'vitest';
import { CORE_STOP_DEADLINE_MS, CoreSupervisor, type CoreStopResult } from './supervisor';

interface StubbornRig {
  readonly sup: CoreSupervisor;
  readonly exit: (code: number | null) => void;
  readonly killed: () => number;
  readonly fire: () => void;
  readonly stderr: (text: string) => void;
}

/** A child that never exits on its own, so the deadline is what decides. */
function stubbornRig(): StubbornRig {
  let onExit: (code: number | null, signal: NodeJS.Signals | null) => void = () => undefined;
  let onStderr: (chunk: string) => void = () => undefined;
  let kills = 0;
  const timers: (() => void)[] = [];
  const sup = new CoreSupervisor({
    binaryPath: '/test-data/core',
    workerPath: null,
    dataDir: '/test-data',
    log: {
      path: '/test-data/core.log',
      write: () => undefined,
      setLevel: () => undefined,
      close: () => undefined,
    },
    spawn: () => ({
      pid: 4242,
      stdin: { end: () => undefined, write: () => true } as unknown as NodeJS.WritableStream,
      stdout: { on: () => undefined } as unknown as NodeJS.ReadableStream,
      stderr: {
        setEncoding: () => undefined,
        on: (event: string, cb: (chunk: string) => void) => {
          if (event === 'data') onStderr = cb;
        },
      } as unknown as NodeJS.ReadableStream,
      kill: () => {
        kills += 1;
      },
      onExit: (cb) => {
        onExit = cb;
      },
      onError: () => undefined,
    }),
    now: () => 0,
    schedule: (fn) => {
      timers.push(fn);
    },
  });
  return {
    sup,
    exit: (code) => {
      onExit(code, null);
    },
    killed: () => kills,
    fire: () => {
      const next = timers.shift();
      next?.();
    },
    stderr: (text) => {
      onStderr(text);
    },
  };
}

describe('CoreSupervisor.stopAndWait', () => {
  it('resolves once the core has actually exited', async () => {
    const rig = stubbornRig();
    rig.sup.start();
    const done: Promise<CoreStopResult> = rig.sup.stopAndWait();
    rig.exit(0);
    expect(await done).toBe('exited');
    expect(rig.killed(), 'an orderly exit is never killed').toBe(0);
  });

  it('kills the core when the deadline passes', async () => {
    const rig = stubbornRig();
    rig.sup.start();
    const done = rig.sup.stopAndWait(CORE_STOP_DEADLINE_MS);
    rig.fire();
    expect(await done).toBe('timed-out');
    expect(rig.killed(), 'a wedged core must not block the installer forever').toBe(1);
  });

  it('resolves immediately when the core was never started', async () => {
    const rig = stubbornRig();
    expect(await rig.sup.stopAndWait()).toBe('not-running');
  });
});

describe('the stderr tail', () => {
  it('reaches the failure detail, so a loader error is diagnosable', () => {
    const rig = stubbornRig();
    rig.sup.start();
    rig.stderr("codotheca-core: /lib/libc.so.6: version 'GLIBC_2.38' not found\n");
    rig.exit(1);
    rig.exit(1);
    const status = rig.sup.status;
    expect(status.kind).toBe('failed');
    if (status.kind === 'failed') expect(status.detail).toMatch(/GLIBC_2\.38/u);
  });
});

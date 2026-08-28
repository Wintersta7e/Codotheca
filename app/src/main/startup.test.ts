import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { describe, expect, it } from 'vitest';
import { type RollingLog, openRollingLog } from './core/log';
import type { CoreStatus } from './core/supervisor';
import { type JoinStep, type StartupDeps, type StartupSupervisor, runStartup } from './startup';

class FakeSupervisor implements StartupSupervisor {
  status: CoreStatus = { kind: 'starting' };
  started = false;
  private cbs: ((s: CoreStatus) => void)[] = [];
  onStatus(fn: (s: CoreStatus) => void): void {
    this.cbs.push(fn);
  }
  start(): void {
    this.started = true;
  }
  set(s: CoreStatus): void {
    this.status = s;
    for (const c of this.cbs) c(s);
  }
}

interface Rig {
  deps: StartupDeps;
  sup: FakeSupervisor;
  order: string[];
  log: RollingLog;
  dir: string;
}

function rig(over: Partial<StartupDeps> = {}): Rig {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'codotheca-start-'));
  const log = openRollingLog({ dir, maxBytes: 1_000_000, keep: 1, level: 'debug' });
  const sup = new FakeSupervisor();
  const order: string[] = [];
  const steps: JoinStep[] = [
    {
      name: 'git-floor',
      run: async () => {
        order.push('git-floor');
        await Promise.resolve();
      },
    },
    {
      name: 'orphan-sessions',
      run: async () => {
        order.push('orphan-sessions');
        await Promise.resolve();
      },
    },
  ];
  const clock = { t: 0 };
  const deps: StartupDeps = {
    acquireInstanceLock: () => true,
    focusExistingWindow: () => {
      order.push('focus');
    },
    dataDir: dir,
    supervisor: sup,
    log,
    now: () => {
      clock.t += 5;
      return clock.t;
    },
    paintUiLane: async () => {
      order.push('paint');
      await Promise.resolve();
    },
    awaitFreeLock: async () => Promise.resolve('free'),
    joinSteps: steps,
    ...over,
  };
  return { deps, sup, order, log, dir };
}

function done(r: Rig): void {
  r.log.close();
  fs.rmSync(r.dir, { recursive: true, force: true });
}

describe('startup', () => {
  it('focuses the first window from a second instance and starts no core', async () => {
    const r = rig({ acquireInstanceLock: () => false });
    const out = await runStartup(r.deps);
    expect(out.kind).toBe('second_instance');
    expect(r.order).toEqual(['focus']);
    expect(r.sup.started).toBe(false);
    done(r);
  });

  it('paints the UI lane before the handshake completes', async () => {
    const r = rig();
    const out = await runStartup(r.deps);
    expect(out.kind).toBe('running');
    expect(r.order).toEqual(['paint']);
    expect(r.sup.started).toBe(true);
    r.sup.set({ kind: 'ready', epoch: 1, coreVersion: '0', protocolVersion: 1, pid: 3 });
    if (out.kind === 'running') expect(await out.joined).toEqual({ kind: 'joined' });
    expect(r.order).toEqual(['paint', 'git-floor', 'orphan-sessions']);
    done(r);
  });

  it('leaves the shelf painted and reports the reason when the core never starts', async () => {
    const r = rig();
    const out = await runStartup(r.deps);
    expect(out.kind).toBe('running');
    r.sup.set({ kind: 'failed', reason: 'spawn', detail: 'noexec', logPath: '/x/codotheca.log' });
    if (out.kind === 'running') {
      expect(await out.joined).toEqual({ kind: 'core_failed', reason: 'spawn' });
    }
    expect(r.order).toEqual(['paint']);
    done(r);
  });

  it('blocks rather than starting a second core when the lock is held', async () => {
    const r = rig({ awaitFreeLock: async () => Promise.resolve('aborted') });
    const out = await runStartup(r.deps);
    expect(out.kind).toBe('blocked');
    expect(r.sup.started).toBe(false);
    done(r);
  });

  it('names a failing join step and stops the sequence', async () => {
    const r = rig();
    r.deps.joinSteps = [
      {
        name: 'git-floor',
        run: async () => {
          await Promise.resolve();
          throw new Error('git too old');
        },
      },
      {
        name: 'orphan-sessions',
        run: async () => {
          r.order.push('orphan-sessions');
          await Promise.resolve();
        },
      },
    ];
    const out = await runStartup(r.deps);
    r.sup.set({ kind: 'ready', epoch: 1, coreVersion: '0', protocolVersion: 1, pid: 3 });
    if (out.kind === 'running') {
      const j = await out.joined;
      expect(j.kind).toBe('step_failed');
      expect(j.kind === 'step_failed' ? j.step : null).toBe('git-floor');
    }
    expect(r.order).not.toContain('orphan-sessions');
    done(r);
  });
});

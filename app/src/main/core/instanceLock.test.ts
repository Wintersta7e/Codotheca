import { spawn } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { describe, expect, it } from 'vitest';
import { CORE_LOCK_FILE, probeCoreLock, waitForCoreLock } from './instanceLock';

function tmp(tag: string): string {
  return fs.mkdtempSync(path.join(os.tmpdir(), `codotheca-${tag}-`));
}
function writeLock(dir: string, pid: number): void {
  fs.writeFileSync(path.join(dir, CORE_LOCK_FILE), JSON.stringify({ pid, started_at: 1 }));
}

describe('core lock probe', () => {
  it('reads a missing lock file as nothing holding the data directory', () => {
    const dir = tmp('nolock');
    expect(probeCoreLock(dir)).toEqual({ held: false, pid: null });
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('reads a lock naming a live process as held', () => {
    const dir = tmp('live');
    writeLock(dir, process.pid);
    expect(probeCoreLock(dir)).toEqual({ held: true, pid: process.pid });
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('reads a lock left by a dead process as stale, not held', async () => {
    const dir = tmp('stale');
    const child = spawn(process.execPath, ['-e', 'setTimeout(() => {}, 60000)']);
    const pid = child.pid ?? 0;
    writeLock(dir, pid);
    expect(probeCoreLock(dir).held).toBe(true);
    child.kill('SIGKILL');
    await new Promise<void>((r) => child.on('exit', () => r()));
    expect(probeCoreLock(dir)).toEqual({ held: false, pid });
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('reads an unparseable lock file as not held, never an error window', () => {
    const dir = tmp('garbage');
    fs.writeFileSync(path.join(dir, CORE_LOCK_FILE), 'not json at all');
    expect(probeCoreLock(dir)).toEqual({ held: false, pid: null });
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it('reports real elapsed milliseconds while waiting, not a spinner', async () => {
    const dir = tmp('wait');
    writeLock(dir, process.pid);
    const reported: number[] = [];
    const clock = { t: 0 };
    const ac = new AbortController();
    const result = await waitForCoreLock(dir, {
      pollMs: 250,
      onWait: (ms) => {
        reported.push(ms);
        if (reported.length === 3) ac.abort();
      },
      signal: ac.signal,
      now: () => clock.t,
      sleep: async () => {
        clock.t += 250;
        await Promise.resolve();
      },
    });
    expect(result).toBe('aborted');
    expect(reported).toEqual([0, 250, 500]);
    fs.rmSync(dir, { recursive: true, force: true });
  });
});

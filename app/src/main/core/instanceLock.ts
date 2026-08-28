/**
 * The advisory lock the core holds for its whole life, probed from the shell.
 *
 * Electron's single-instance lock is held by the shell, so it cannot see a core orphaned by a
 * previous crash — which is the case it does not cover and this one does. The core holds the
 * OS lock; the shell reads the pid beside it and asks the OS whether that process is alive.
 * The shell never opens the database.
 */
import * as fs from 'node:fs';
import * as path from 'node:path';

export const CORE_LOCK_FILE = 'core.lock';
export const LOCK_WAIT_POLL_MS = 250;
/** §11.2a: `FORCE` appears only after ten seconds of actual waiting. */
export const FORCE_OFFERED_AFTER_MS = 10_000;

export interface CoreLockProbe {
  held: boolean;
  pid: number | null;
}

function readLockPid(dataDir: string): number | null {
  let raw: string;
  try {
    raw = fs.readFileSync(path.join(dataDir, CORE_LOCK_FILE), 'utf8');
  } catch {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof parsed !== 'object' || parsed === null) return null;
  const pid: unknown = (parsed as { pid?: unknown }).pid;
  return typeof pid === 'number' && Number.isInteger(pid) && pid > 0 ? pid : null;
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (e: unknown) {
    // EPERM means the process exists and belongs to someone else. That is alive.
    return (e as NodeJS.ErrnoException).code === 'EPERM';
  }
}

/** Never opens the database. The core owns the only connection of any kind. */
export function probeCoreLock(dataDir: string): CoreLockProbe {
  const pid = readLockPid(dataDir);
  if (pid === null) return { held: false, pid: null };
  return alive(pid) ? { held: true, pid } : { held: false, pid };
}

export interface WaitOptions {
  pollMs: number;
  onWait: (elapsedMs: number) => void;
  signal: AbortSignal;
  now: () => number;
  sleep: (ms: number) => Promise<void>;
}

/** Polls until the lock is free or the caller aborts, reporting real elapsed milliseconds. */
export async function waitForCoreLock(
  dataDir: string,
  opts: WaitOptions,
): Promise<'free' | 'aborted'> {
  const started = opts.now();
  for (;;) {
    if (!probeCoreLock(dataDir).held) return 'free';
    if (opts.signal.aborted) return 'aborted';
    opts.onWait(opts.now() - started);
    await opts.sleep(opts.pollMs);
  }
}

/** The `FORCE` control. Kills the process named in the lock file and nothing else. */
export function forceReleaseCoreLock(dataDir: string): boolean {
  const probe = probeCoreLock(dataDir);
  if (!probe.held || probe.pid === null) return false;
  try {
    process.kill(probe.pid, 'SIGKILL');
    return true;
  } catch {
    return false;
  }
}

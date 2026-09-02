/**
 * The one place a core process is created. Everything else takes a `CoreChild`, so the whole
 * lifecycle is testable without a binary.
 */
import { spawn } from 'node:child_process';

export interface CoreArgv {
  binaryPath: string;
  dataDir: string;
  epoch: number;
  parentPid: number;
  /**
   * §13's in-distro worker, or `null` where this build has none.
   *
   * The shell computes it (`resolveWorkerBinary`) and passes it, rather than the core deriving
   * a second convention for the same file: one value spelled in two languages is one value that
   * drifts. `null` and a path that does not exist are the same answer here — no distro can be
   * scanned — and the core says so rather than reporting an empty distro.
   */
  workerPath: string | null;
}

export interface CoreChild {
  readonly pid: number | null;
  readonly stdin: NodeJS.WritableStream;
  readonly stdout: NodeJS.ReadableStream;
  readonly stderr: NodeJS.ReadableStream;
  kill(): void;
  onExit(cb: (code: number | null, signal: NodeJS.Signals | null) => void): void;
  onError(cb: (err: Error) => void): void;
}

export type SpawnCore = (argv: CoreArgv) => CoreChild;

/**
 * The data directory is decided by the shell and passed in argv: Electron's `userData` and
 * Rust's own directory conventions disagree on Linux, and letting each side pick is two
 * databases.
 */
/**
 * The core's argv, exported so a test asserts the argv the product actually builds rather than
 * a second copy of the rule.
 */
export function coreArguments(argv: CoreArgv): string[] {
  const args = [
    `--data-dir=${argv.dataDir}`,
    `--epoch=${String(argv.epoch)}`,
    `--parent-pid=${String(argv.parentPid)}`,
  ];
  // Omitted rather than passed empty: the core rejects `--worker=` with no value, because an
  // empty one would mean the shell thought it had a path and did not.
  if (argv.workerPath !== null) {
    args.push(`--worker=${argv.workerPath}`);
  }
  return args;
}

export const spawnCoreChild: SpawnCore = (argv) => {
  const child = spawn(argv.binaryPath, coreArguments(argv), {
    stdio: ['pipe', 'pipe', 'pipe'],
    windowsHide: true,
  });
  return {
    pid: child.pid ?? null,
    stdin: child.stdin,
    stdout: child.stdout,
    stderr: child.stderr,
    kill: (): void => {
      child.kill();
    },
    onExit: (cb): void => {
      child.on('exit', cb);
    },
    onError: (cb): void => {
      child.on('error', cb);
    },
  };
};

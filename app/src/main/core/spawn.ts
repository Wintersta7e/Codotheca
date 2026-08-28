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
export const spawnCoreChild: SpawnCore = (argv) => {
  const child = spawn(
    argv.binaryPath,
    [
      `--data-dir=${argv.dataDir}`,
      `--epoch=${String(argv.epoch)}`,
      `--parent-pid=${String(argv.parentPid)}`,
    ],
    { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true },
  );
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

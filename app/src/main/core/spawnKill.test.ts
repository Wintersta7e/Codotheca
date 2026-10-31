import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { describe, expect, it } from 'vitest';
import { spawnCoreChild } from './spawn';

/**
 * The core consumes SIGTERM on Unix so that a signal sent to the shell's whole process group
 * leaves its stop to the shell. The shell's own kill must therefore not be a SIGTERM, or every
 * kill the supervisor issues — a protocol kill, the stop deadline — would reach a core that
 * ignores it and lives on.
 */
describe.skipIf(process.platform === 'win32')('the shell’s kill of the core', () => {
  it('is a SIGKILL, which a core consuming SIGTERM still dies of', async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'codotheca-kill-'));
    const binary = path.join(dir, 'core');
    // `trap ''` ignores the signal, and an ignored signal stays ignored across `exec`.
    fs.writeFileSync(binary, "#!/bin/sh\ntrap '' TERM\necho ready\nexec sleep 30\n", {
      mode: 0o755,
    });
    const child = spawnCoreChild({
      binaryPath: binary,
      dataDir: dir,
      epoch: 1,
      parentPid: process.pid,
      workerPath: null,
      rebuild: false,
    });
    const exited = new Promise<NodeJS.Signals | null>((resolve) => {
      child.onExit((_code, signal) => {
        resolve(signal);
      });
    });
    await new Promise<void>((resolve) => {
      child.stdout.once('data', () => {
        resolve();
      });
    });

    child.kill();
    const signal = await Promise.race([
      exited,
      new Promise<string>((resolve) => setTimeout(resolve, 3_000, 'still running after 3 s')),
    ]);
    expect(signal).toBe('SIGKILL');
    fs.rmSync(dir, { recursive: true, force: true });
  });
});

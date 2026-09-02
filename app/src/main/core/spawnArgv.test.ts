import { describe, expect, it } from 'vitest';
import { type CoreArgv, coreArguments } from './spawn';

/**
 * §13's worker path reaches the core through argv rather than through a second convention in
 * Rust: the shell already computes it (`resolveWorkerBinary`), and one value spelled in two
 * languages is one value that drifts.
 */
describe('the core argv', () => {
  const base = { binaryPath: '/c/core', dataDir: '/c/data', epoch: 3, parentPid: 42 };

  it('omits the worker flag entirely when this build staged none', () => {
    const args = coreArguments({ ...base, workerPath: null } satisfies CoreArgv);
    expect(args).toEqual(['--data-dir=/c/data', '--epoch=3', '--parent-pid=42']);
    // The core refuses an empty value, so passing one would take the process down at startup
    // rather than leaving distros unscanned.
    expect(args.some((a) => a === '--worker=')).toBe(false);
  });

  it('passes the staged worker, without which no distro can be scanned at all', () => {
    const worker = '/c/res/worker/linux-x64/codotheca-worker';
    expect(coreArguments({ ...base, workerPath: worker } satisfies CoreArgv)).toContain(
      `--worker=${worker}`,
    );
  });
});

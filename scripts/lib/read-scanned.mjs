import { readFileSync } from 'node:fs';

/**
 * Read a file a gate has just walked to, or `null` if it is no longer there.
 *
 * Every gate under `scripts/` walks a directory and reads the collected paths afterwards, and
 * several tests plant a probe file inside those same directories to prove their own gate can
 * fail — `app/test/styleGates.test.ts` plants `app/src/renderer/styles/__probe.css`, and
 * `app/test/destructiveTokens.test.ts` plants two under `app/src/renderer`. Vitest runs the node
 * project's files in parallel, so a walked path can be gone by the time it is read, and an
 * unguarded `readFileSync` crashes the gate with an ENOENT stack instead of reporting. **A gate
 * that throws is a gate that reports nothing**, and it takes the whole suite red with it.
 *
 * A file that is no longer there carries nothing to check, so it is skipped — and callers must
 * not count it, or the "scanned nothing" guard each gate carries stops meaning what it says. Any
 * other read error is a real problem and is raised.
 */
export function readScannedFile(path) {
  try {
    return readFileSync(path, 'utf8');
  } catch (error) {
    if (error && error.code === 'ENOENT') return null;
    throw error;
  }
}

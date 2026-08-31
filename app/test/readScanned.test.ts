import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const REPO = fileURLToPath(new URL('../..', import.meta.url));
const HELPER = join(REPO, 'scripts/lib/read-scanned.mjs');

interface Helper {
  readonly readScannedFile: (path: string) => string | null;
}

async function importHelper(): Promise<Helper> {
  const module: unknown = await import(/* @vite-ignore */ pathToFileURL(HELPER).href);
  if (
    typeof module !== 'object' ||
    module === null ||
    !('readScannedFile' in module) ||
    typeof module.readScannedFile !== 'function'
  ) {
    throw new Error('read-scanned.mjs does not export readScannedFile');
  }
  return module as unknown as Helper;
}

let dir = '';
beforeAll(() => {
  dir = mkdtempSync(join(tmpdir(), 'read-scanned-'));
});
afterAll(() => {
  rmSync(dir, { recursive: true, force: true });
});

/**
 * Every gate under `scripts/` walks a directory and reads the collected paths afterwards, and
 * three test files plant probes inside those directories to prove their own gate can fail. Vitest
 * runs them in parallel, so a walked path can be gone by the time it is read. This was not
 * hypothetical: it crashed two gates with ENOENT stacks and took the whole app suite red, once in
 * each direction. Racing it is far too narrow a window to test, so the seam is tested instead.
 */
describe('readScannedFile', () => {
  it('returns the text of a file that is still there', async () => {
    const { readScannedFile } = await importHelper();
    const present = join(dir, 'present.css');
    writeFileSync(present, '.x { color: #ffffff; }\n', 'utf8');
    expect(readScannedFile(present)).toContain('#ffffff');
  });

  it('returns null for a file that vanished between the walk and the read', async () => {
    const { readScannedFile } = await importHelper();
    const vanished = join(dir, 'vanished.css');
    writeFileSync(vanished, '.y {}\n', 'utf8');
    rmSync(vanished, { force: true });
    expect(readScannedFile(vanished)).toBeNull();
  });

  it('raises anything that is not a missing file, rather than swallowing it', async () => {
    // The guard is narrow on purpose: a catch-all would turn an unreadable file into a silently
    // skipped one, and a gate that skips what it cannot read is a gate that passes on nothing.
    const { readScannedFile } = await importHelper();
    expect(() => readScannedFile(dir)).toThrow();
  });
});

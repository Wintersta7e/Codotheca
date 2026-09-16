import { readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';

/**
 * Two gates over one tree.
 *
 * **The caller gate (§25.8).** *"The shell is its only caller in phase 1."* `remote.webUrl` is
 * not privileged — `protocol/lib/schema.mjs` throws on a privileged command that carries no
 * `Bytes` — so the bridge will answer it for the renderer, and nothing about the capability
 * enforces that sentence. This does. An earlier revision of the plan asserted that
 * `isRendererCallable` was not widened; that was false, and nothing checked it, which is the
 * shape of a bar written past its defect.
 *
 * Both halves read every file through `scripts/lib/read-scanned.mjs`'s contract: other gates
 * plant probe files inside `app/src/renderer/` while vitest runs the node project in parallel,
 * so a walked path can be gone by the time it is read, and an unguarded read throws a raw ENOENT
 * that takes the whole suite red. A vanished file is skipped **before** it is counted, or the
 * "scanned nothing" guard stops meaning what it says.
 */
const appDir = fileURLToPath(new URL('..', import.meta.url));
const srcDir = join(appDir, 'src');

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    let stat;
    try {
      stat = statSync(path);
    } catch (error: unknown) {
      if (isMissing(error)) continue;
      throw error;
    }
    if (stat.isDirectory()) walk(path, out);
    else if (/\.tsx?$/u.test(entry) && !/\.test\.tsx?$/u.test(entry)) out.push(path);
  }
  return out;
}

function isMissing(error: unknown): boolean {
  return typeof error === 'object' && error !== null && 'code' in error && error.code === 'ENOENT';
}

const files: readonly (readonly [string, string])[] = walk(srcDir).flatMap((path) => {
  const text = readScannedFile(path);
  return text === null ? [] : [[relative(appDir, path).replace(/\\/gu, '/'), text] as const];
});

function matching(pattern: RegExp): string[] {
  return files
    .filter(([, text]) => pattern.test(text))
    .map(([path]) => path)
    .sort();
}

describe('§25.8: the shell is `remote.webUrl`s only caller', () => {
  it('scanned a real tree, or every assertion below is vacuous', () => {
    process.stderr.write(
      `remoteScope: caller gate scanned ${String(files.length)} source file(s)\n`,
    );
    expect(files.length, 'the caller gate scanned nothing, so it proved nothing').toBeGreaterThan(
      40,
    );
  });

  /**
   * Four files may name it and the list is exact, not a bound. Three of them are tables rather
   * than callers: `src/generated/protocol.ts` is the contract and declares every command,
   * `src/main/core/idempotence.ts` classifies every command as a read or a write, and
   * `src/main/index.ts` is `KNOWN_COMMANDS`, the list the bridge will accept. The opener is the
   * only thing that issues it.
   */
  it('names the command in exactly four files, and only one of them calls it', () => {
    expect(matching(/remote\.webUrl/u)).toEqual([
      'src/generated/protocol.ts',
      'src/main/core/idempotence.ts',
      'src/main/dialogs/externalLink.ts',
      'src/main/index.ts',
    ]);
  });

  it('names it in no renderer module at all', () => {
    const renderer = files.filter(([path]) => path.startsWith('src/renderer/'));
    expect(renderer.length, 'the renderer half of the walk found nothing').toBeGreaterThan(20);
    expect(renderer.filter(([, text]) => /remote\.webUrl/u.test(text)).map(([p]) => p)).toEqual([]);
  });

  it('calls shell.openExternal from the main process only', () => {
    expect(matching(/openExternal/u).filter((p) => p.startsWith('src/renderer/'))).toEqual([]);
  });
});

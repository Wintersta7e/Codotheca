import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * §5.6's scope is the half that makes the note admissible: it renders only inside an opened
 * project — never on the grid, never in Peek, never in the list, never in the palette, never in
 * triage. That is the cheap half to automate, so it is automated rather than remembered.
 *
 * This lives in the node project because the renderer project carries no Node types by design.
 */
const appDir = fileURLToPath(new URL('..', import.meta.url));
const srcDir = join(appDir, 'src');

function sources(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    let dirent;
    try {
      dirent = statSync(path);
    } catch (error) {
      // A probe another gate planted can vanish between the walk and the stat.
      if (isMissing(error)) continue;
      throw error;
    }
    if (dirent.isDirectory()) sources(path, out);
    else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry)) out.push(path);
  }
  return out;
}

function isMissing(error: unknown): boolean {
  return typeof error === 'object' && error !== null && 'code' in error && error.code === 'ENOENT';
}

/**
 * Every file this gate walked, read **once**, with the vanished ones dropped before they are
 * counted — the contract `scripts/lib/read-scanned.mjs` states for the four `scripts/` gates.
 *
 * `app/test/styleGates.test.ts` and `app/test/destructiveTokens.test.ts` plant probe files inside
 * `app/src/renderer/` to prove their own gates can fail, and vitest runs the node project in
 * parallel, so a walked path can be gone by the time it is read. An unguarded `readFileSync`
 * throws a raw ENOENT that takes the **whole suite** red rather than failing this one assertion.
 *
 * Skipping *before* counting is the load-bearing half: the `files.length` guard below exists to
 * prove this gate scanned a real tree, and counting a file that was never read would make that
 * guard stop meaning what it says.
 */
const files: readonly (readonly [string, string])[] = sources(srcDir).flatMap((path) => {
  try {
    return [[path, readFileSync(path, 'utf8')] as const];
  } catch (error) {
    if (isMissing(error)) return [];
    throw error;
  }
});

function importersOf(pattern: RegExp): string[] {
  return files
    .filter(([, text]) => pattern.test(text))
    .map(([path]) => relative(appDir, path).replace(/\\/g, '/'))
    .sort();
}

describe('§5.6 scope: a roast renders only inside an opened project card', () => {
  it('scans a real tree, or every assertion below is vacuous', () => {
    expect(files.length).toBeGreaterThan(40);
  });

  it('is imported by exactly one module, and that module is the project page note', () => {
    expect(importersOf(/from '[^']*derive\/roast'/)).toEqual([
      'src/renderer/project/RoastNote.tsx',
    ]);
  });

  it('the note module is imported by exactly one module, and that is the page shell', () => {
    expect(importersOf(/from '[^']*\/RoastNote'/)).toEqual([
      'src/renderer/project/ProjectPage.tsx',
    ]);
  });
});

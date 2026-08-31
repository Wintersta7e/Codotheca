import { readdirSync, readFileSync, statSync } from 'node:fs';
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
    if (statSync(path).isDirectory()) sources(path, out);
    else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry)) out.push(path);
  }
  return out;
}

const files = sources(srcDir);

function importersOf(pattern: RegExp): string[] {
  return files
    .filter((path) => pattern.test(readFileSync(path, 'utf8')))
    .map((path) => relative(appDir, path).replace(/\\/g, '/'))
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

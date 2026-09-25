import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * §2.4's copy boundary, held mechanically: **the core's `message` is diagnostic and is never
 * rendered.** That rule is invisible in a passing render test, because a build that renders
 * `err.message` renders *something* and every assertion about layout still passes. The cheap
 * half is a grep, so it is a grep.
 *
 * It lives in the node project because the renderer project carries no Node types by design —
 * the same reason `roastScope.test.ts` lives here.
 */
const appDir = fileURLToPath(new URL('..', import.meta.url));
const srcDir = join(appDir, 'src');

/**
 * Several tests in this directory plant probe files inside `app/src/renderer` to prove their
 * own gate can fail, and vitest runs the node project's files in parallel — so a walked path
 * can be gone by the time it is read. A vanished file is skipped **before it is counted**, or
 * the "scanned nothing" guards below stop meaning what they say.
 */
function sources(dir: string, out: string[] = []): string[] {
  let entries: string[];
  try {
    entries = readdirSync(dir);
  } catch {
    return out;
  }
  for (const entry of entries) {
    const path = join(dir, entry);
    let isDir: boolean;
    try {
      isDir = statSync(path).isDirectory();
    } catch {
      continue;
    }
    if (isDir) sources(path, out);
    else if (/\.tsx?$/.test(entry) && !/\.test\.tsx?$/.test(entry)) out.push(path);
  }
  return out;
}

interface Scanned {
  readonly path: string;
  readonly text: string;
  /** The same text with comments blanked out, so a comment *about* a ban does not trip it. */
  readonly code: string;
}

/**
 * Blanks comments while preserving offsets. Grepping raw source for a banned token matches the
 * prose explaining the ban — that has produced a wrong ruling in this repo twice — and
 * `scripts/check-destructive-tokens.mjs` strips comments for exactly this reason.
 */
function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\r\n]/g, ' '))
    .replace(
      /(^|[^:])\/\/[^\r\n]*/g,
      (comment, lead: string) => lead + ' '.repeat(comment.length - lead.length),
    );
}

function scan(paths: readonly string[]): Scanned[] {
  const out: Scanned[] = [];
  for (const path of paths) {
    let text: string;
    try {
      text = readFileSync(path, 'utf8');
    } catch {
      continue;
    }
    out.push({ path, text, code: stripComments(text) });
  }
  return out;
}

const rel = (path: string): string => relative(appDir, path).replace(/\\/g, '/');

const SURFACE_DIRS = ['errors', 'summary', 'failure', 'notices'].map((d) =>
  join(srcDir, 'renderer', d),
);
const surfaces = scan(SURFACE_DIRS.flatMap((dir) => sources(dir)));

describe('the comment stripper the greps below depend on', () => {
  it('blanks a comment and keeps the code beside it', () => {
    expect(stripComments("// FORGET\nconst a = 'b';")).not.toContain('FORGET');
    expect(stripComments("/* err.message */\nconst a = 'b';")).toContain("const a = 'b';");
    expect(stripComments("const u = 'https://x';")).toContain('https://x');
  });
});

describe('§2.4: the core’s message is never rendered', () => {
  it('scans every module of these four surfaces, or the assertion below is vacuous', () => {
    expect(surfaces.map((s) => rel(s.path)).sort()).toEqual([
      'src/renderer/errors/ErrorExplained.tsx',
      'src/renderer/errors/errorKind.ts',
      'src/renderer/failure/FailureWindow.tsx',
      'src/renderer/failure/copy.ts',
      'src/renderer/notices/copy.ts',
      'src/renderer/summary/ScanSummary.tsx',
      'src/renderer/summary/copy.ts',
    ]);
  });

  it('is reached for by no module in them', () => {
    const offenders = surfaces
      .filter((s) => /\.message\b|BridgeError|CoreError\b/.test(s.code))
      .map((s) => rel(s.path));
    expect(offenders).toEqual([]);
  });
});

describe('one never-succeeded predicate in the tree', () => {
  const all = scan(sources(srcDir));

  it('walks a real tree, or the assertion below is vacuous', () => {
    expect(all.length).toBeGreaterThan(40);
  });

  it('is declared exactly once, and in the error module', () => {
    const declarers = all
      .filter((s) => /export function neverSucceeded|function neverSucceeded\(/.test(s.code))
      .map((s) => rel(s.path));
    expect(declarers).toEqual(['src/renderer/errors/errorKind.ts']);
  });
});

describe('§17: no destructive token reaches a rendered string', () => {
  it('does not appear in any module of these four surfaces', () => {
    const offenders = surfaces.filter((s) => s.code.includes('FORGET')).map((s) => rel(s.path));
    expect(offenders).toEqual([]);
  });
});

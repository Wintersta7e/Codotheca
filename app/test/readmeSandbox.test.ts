import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * §25.5's frame exists to make a sanitiser bypass worth nothing, and one attribute is the whole
 * of it: `sandbox` present and **exactly empty**. `allow-scripts` would resurrect script
 * execution; `allow-same-origin` would give the frame the parent's origin and with it the bridge.
 *
 * So the strings appear **nowhere** in the panel's sources. That is a weaker claim than the
 * rendered attribute — which `frame.test.ts` asserts — and it catches the thing a rendered
 * assertion cannot: a flag added to a second code path nothing happens to render in a test.
 *
 * It lives in the node project because the renderer project carries no Node types by design
 * (`tsconfig.web.json` gives it `types: ["vite/client"]` and nothing else).
 */
const appDir = fileURLToPath(new URL('..', import.meta.url));
const panelDir = join(appDir, 'src/renderer/project/readme');

function isMissing(error: unknown): boolean {
  return typeof error === 'object' && error !== null && 'code' in error && error.code === 'ENOENT';
}

function sources(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    let stat;
    try {
      stat = statSync(path);
    } catch (error) {
      // A probe another gate planted can vanish between the walk and the stat.
      if (isMissing(error)) continue;
      throw error;
    }
    if (stat.isDirectory()) sources(path, out);
    // Product sources only. A test that asserts a flag is absent has to name it, which is why
    // `roastScope.test.ts` excludes its own kind from the same walk.
    else if (/\.tsx?$/u.test(entry) && !/\.test\.tsx?$/u.test(entry)) out.push(path);
  }
  return out;
}

/**
 * Every file this gate walked, read once, with the vanished ones dropped **before** they are
 * counted — the contract `scripts/lib/read-scanned.mjs` states. Counting a file that was never
 * read would make the "scanned nothing" guard below stop meaning what it says.
 */
const files: readonly (readonly [string, string])[] = sources(panelDir).flatMap((path) => {
  try {
    return [[path, readFileSync(path, 'utf8')] as const];
  } catch (error) {
    if (isMissing(error)) return [];
    throw error;
  }
});

/** Comments removed, so a module may **say** what it does not do. */
function codeOnly(text: string): string {
  return text
    .replace(/\/\*[\s\S]*?\*\//gu, '')
    .split('\n')
    .filter((line) => !line.trim().startsWith('//'))
    .join('\n');
}

describe('the README panel sources', () => {
  it('scanned a real tree, or every assertion below is vacuous', () => {
    process.stderr.write(`readmeSandbox: scanned ${String(files.length)} file(s)\n`);
    expect(files.length, 'the panel directory is empty').toBeGreaterThan(0);
  });

  it('name neither sandbox flag, anywhere', () => {
    const hits: string[] = [];
    for (const [path, text] of files) {
      const code = codeOnly(text);
      for (const flag of ['allow-scripts', 'allow-same-origin']) {
        if (code.includes(flag))
          hits.push(`${relative(appDir, path).replace(/\\/gu, '/')}: ${flag}`);
      }
    }
    process.stderr.write(
      `readmeSandbox: ${String(files.length)} file(s) scanned for 2 flags, ${String(hits.length)} hit(s)\n`,
    );
    expect(hits).toEqual([]);
  });

  it('never guess a language: highlightAuto is named in no module', () => {
    // `highlight.ts`'s own header forbids it by name, which is why this reads code and not prose.
    const hits = files
      .filter(([, text]) => codeOnly(text).includes('highlightAuto'))
      .map(([path]) => relative(appDir, path).replace(/\\/gu, '/'));
    process.stderr.write(
      `readmeSandbox: ${String(files.length)} file(s) scanned for highlightAuto, ${String(hits.length)} hit(s)\n`,
    );
    expect(hits).toEqual([]);
  });

  it('reconfigure DOMPurify for no data: URI, and build no srcdoc by hand', () => {
    // The substitution happens after the sanitiser, through the DOM API, on nodes it produced.
    // Widening `ALLOWED_URI_REGEXP` is how `data:text/html` in an href becomes XSS.
    const hits = files
      .filter(([, text]) => /ALLOWED_URI_REGEXP|ADD_URI_SAFE_ATTR/u.test(codeOnly(text)))
      .map(([path]) => relative(appDir, path).replace(/\\/gu, '/'));
    expect(hits).toEqual([]);
  });
});

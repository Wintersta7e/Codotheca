import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test, expect } from 'vitest';
import { CONTENT_SCAN_LANGUAGES } from '../src/shared/contentScan';
import { required } from '../src/shared/required';

const REPO_ROOT = fileURLToPath(new URL('../..', import.meta.url));

/**
 * The distinct `prog(...)` language names of the core's `BY_EXT`, sorted.
 *
 * §29.2's rule 3 is that flag, not a second list — so this reads the flag rather than a list of
 * names, and an entry switched from `markup` to `prog` moves this side on its own.
 */
function coreProgrammingLanguages(): string[] {
  const source = readFileSync(join(REPO_ROOT, 'core/src/jobs/classify.rs'), 'utf8');
  const start = source.indexOf('const BY_EXT');
  expect(start, 'BY_EXT is not where this test expects it').not.toBe(-1);
  const open = source.indexOf('[', source.indexOf('=', start));
  const close = source.indexOf('];', open);
  const body = source.slice(open + 1, close);
  const names = [...body.matchAll(/prog\("((?:[^"\\]|\\.)*)"\)/g)].map((m) =>
    required(m[1], 'language name').replace(/\\(.)/g, '$1'),
  );
  return [...new Set(names)].sort();
}

// AC-P3-29-17. §29.2's rule 3 gives `BY_EXT` a second reader, and it was written for the
// language byte census: someone adding an extension there to make a language appear in the
// language bar would otherwise silently widen what this product reads off the user's disk.
// Rendering the list makes that a visible change to a stated policy rather than a table edit.
test('AC-P3-29-17 the rendered language list is the core programming-language set', () => {
  const fromCore = coreProgrammingLanguages();
  const rendered = [...CONTENT_SCAN_LANGUAGES];
  // Both counts are derived and printed, and the gate fails at zero on either side.
  process.stderr.write(
    `contentScanLanguages: core declares ${String(fromCore.length)} programming language(s); ` +
      `the consent surface renders ${String(rendered.length)}\n`,
  );
  expect(fromCore.length).toBeGreaterThan(0);
  expect(rendered.length).toBeGreaterThan(0);
  expect(rendered).toEqual(fromCore);
});

// The markup entries are what rule 3 excludes: a checklist in a README is not debt, and a
// lockfile is read by name elsewhere. None of them may appear on this surface.
test('AC-P3-29-17 no markup language is rendered as one this scan reads', () => {
  const source = readFileSync(join(REPO_ROOT, 'core/src/jobs/classify.rs'), 'utf8');
  const markup = [...source.matchAll(/markup\("((?:[^"\\]|\\.)*)"\)/g)].map((m) =>
    required(m[1], 'markup name'),
  );
  process.stderr.write(
    `contentScanLanguages: core declares ${String(markup.length)} markup entr(ies)\n`,
  );
  expect(markup.length).toBeGreaterThan(0);
  for (const name of new Set(markup)) {
    expect(CONTENT_SCAN_LANGUAGES as readonly string[]).not.toContain(name);
  }
});

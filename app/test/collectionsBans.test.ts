import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * §8.8's four bans, scanned over every source in `src/renderer/collections/`.
 *
 * It lives here rather than beside the code it polices because `tsconfig.web.json` types the
 * renderer with `vite/client` alone — `node:fs` has no types there and eslint reports every call
 * on it as unsafe. `test/` is the node project, which is where this repo already keeps its
 * source-scanning gates. The scan is a directory walk and not a list of imports, so a file added
 * to that directory tomorrow is covered without anyone remembering to add it.
 */
const DIR = fileURLToPath(new URL('../src/renderer/collections/', import.meta.url));
const sources = readdirSync(DIR).filter((name) => !name.includes('.test.'));
const allText = sources.map((name) => readFileSync(join(DIR, name), 'utf8')).join('\n');

describe('the collections bans', () => {
  // A gate whose passing run reads nothing is a failing gate that looks green.
  it('actually read the directory', () => {
    expect(sources.length).toBeGreaterThanOrEqual(9);
    expect(allText.length).toBeGreaterThan(5_000);
    expect(sources).toContain('collections.css');
    expect(sources).toContain('CollectionChip.tsx');
  });

  // The nearest thing phase 1 has to a destructive control is the remove control, and it removes
  // a saved query: the `collection` row and its `collection_member` rows, and nothing else.
  it('never spells the word phase 1 has no operation for', () => {
    expect(allText).not.toMatch(/FORGET/);
  });

  // §5.6 confines the dry one-line note to an opened project page — never the grid, never Peek,
  // never the list, never the palette, never triage, and never a chip.
  it('carries no roast', () => {
    for (const spelling of ['roast', 'Roast', 'ROAST']) expect(allText).not.toContain(spelling);
  });

  // §8.7: decision-carrying text is `--text-3` or lighter. `--text-4` and `--text-5` are
  // ornament only, and this stylesheet carries no ornament.
  it('sets no decision-carrying text below the floor', () => {
    const css = readFileSync(join(DIR, 'collections.css'), 'utf8');
    expect(css.length).toBeGreaterThan(400);
    expect(css).not.toContain('--text-4');
    expect(css).not.toContain('--text-5');
  });

  // Criterion 9's negative half: a saved query survives a restart because it is a row the core
  // owns, re-read through `collections.list`. Nothing here mirrors one into `view_state`, which
  // would be a second store for the same fact and free to disagree with the first.
  it('keeps no collection state in view_state', () => {
    expect(allText).not.toContain('view.set');
    expect(allText).not.toContain('view_state');
  });
});

import { readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';

/**
 * [p3] §30.13's source audits. **Each prints its count and fails at zero** — a gate whose passing
 * run scans nothing is a failing gate.
 *
 * All four read through `scripts/lib/read-scanned.mjs`'s contract: a file that vanished between
 * the walk and the read is skipped **before it is counted**, or the *"scanned nothing"* guard
 * stops meaning what it says. `app/test/styleGates.test.ts` and
 * `app/test/destructiveTokens.test.ts` plant probe files inside `app/src/renderer/` while vitest
 * runs this project in parallel, which is why that is not hypothetical.
 *
 * They live in the **node** vitest project, which is already one of the four acceptance capture
 * writers, so no new capture writer is created and no gate step goes unwired.
 */
const appDir = fileURLToPath(new URL('..', import.meta.url));
const repoDir = join(appDir, '..');

function isMissing(error: unknown): boolean {
  return typeof error === 'object' && error !== null && 'code' in error && error.code === 'ENOENT';
}

function walk(dir: string, match: RegExp, out: string[] = []): string[] {
  let entries: string[];
  try {
    entries = readdirSync(dir);
  } catch (error) {
    if (isMissing(error)) return out;
    throw error;
  }
  for (const entry of entries) {
    const path = join(dir, entry);
    let dirent;
    try {
      dirent = statSync(path);
    } catch (error) {
      if (isMissing(error)) continue;
      throw error;
    }
    if (dirent.isDirectory()) walk(path, match, out);
    else if (match.test(entry)) out.push(path);
  }
  return out;
}

/** Walked paths, read once, **with the vanished ones dropped before they are counted.** */
function readAll(paths: readonly string[]): (readonly [string, string])[] {
  return paths.flatMap((path) => {
    const text: string | null = readScannedFile(path);
    return text === null ? [] : [[path, text] as const];
  });
}

const rendererFiles = readAll(walk(join(appDir, 'src', 'renderer'), /\.tsx?$/));
const coreFiles = readAll(walk(join(repoDir, 'core', 'src'), /\.rs$/));

function rel(path: string): string {
  return relative(repoDir, path).replace(/\\/gu, '/');
}

describe('§30.13 audit 1 — two units, never one', () => {
  /**
   * **A12b.** `scoredOpen` counts items and the basis counts checks. Each field carries its unit
   * in its name and neither is derived from the other; a single *value* over the two is the thing
   * §30.1 exists to prevent, and it arrives as arithmetic between the two.
   */
  it('AC-P3-30-2 no surface combines a check count and an item count into one value', () => {
    // eslint-disable-next-line no-console
    console.log(
      `AC-P3-30-2 scanned ${rendererFiles.length} renderer file(s) and ${coreFiles.length} core file(s)`,
    );
    expect(rendererFiles.length).toBeGreaterThan(0);
    expect(coreFiles.length).toBeGreaterThan(0);

    // `scoredOpen`/`scored_open` in any arithmetic with a basis term. The basis fields are the
    // check-counting half, so an expression naming both is the combination itself.
    const basisTerms = ['ran', 'eligible', 'unknown', 'off', 'notApplicable', 'not_applicable'];
    const offenders: string[] = [];
    for (const [path, text] of [...rendererFiles, ...coreFiles]) {
      for (const [i, line] of text.split('\n').entries()) {
        const code = line.trim();
        if (code.startsWith('//') || code.startsWith('*') || code.startsWith('///')) continue;
        if (!/scoredOpen|scored_open/u.test(code)) continue;
        if (!/[+\-*/%]/u.test(code)) continue;
        if (!basisTerms.some((term) => new RegExp(`\\b${term}\\b`, 'u').test(code))) continue;
        offenders.push(`${rel(path)}:${i + 1}`);
      }
    }
    expect(offenders).toEqual([]);

    // The schema half: `HealthBasis` holds no item count, asserted against the schema rather than
    // against a remembered field list.
    const raw: string | null = readScannedFile(join(repoDir, 'protocol/schema/protocol.json'));
    expect(raw).not.toBeNull();
    const schema = JSON.parse(raw ?? '{}') as {
      types: Record<string, { fields?: Record<string, string> }>;
    };
    const basisFields = Object.keys(schema.types['HealthBasis']?.fields ?? {});
    expect(basisFields.length).toBeGreaterThan(0);
    for (const field of basisFields) {
      expect(field.toLowerCase()).not.toMatch(/open|item|unverified/u);
    }
  });
});

describe('§30.13 audit 2 — no second freeze gate', () => {
  /**
   * **§30.2's closing rule.** The freeze is applied once, upstream: §30 hands down the reading,
   * the per-check outcomes and the open-item set, and **no consumer re-derives `absent`,
   * `suppressed` or Reference exclusion.** `HealthState` says which reading a surface is looking
   * at; it is not a flag a consumer acts on.
   *
   * §33 and §35 land in wave 4, so this asserts the rule over the tree it has — and it is what
   * those plans are then held to.
   */
  it('AC-P3-30-11a no renderer consumer re-derives the freeze or the exclusions', () => {
    // eslint-disable-next-line no-console
    console.log(`AC-P3-30-11a scanned ${rendererFiles.length} renderer file(s)`);
    expect(rendererFiles.length).toBeGreaterThan(0);

    // Two surfaces are permitted to read the state, and neither re-derives what it means:
    // §30.7's `tabsFor` makes a **mount** decision from it, and §30's own tab renders the reading
    // it was handed. Everything else is a consumer, and a consumer branching on `HealthState` is
    // the second freeze gate this audit exists to keep out — §33 renders what it is handed and
    // §35 ranks what it is handed.
    const allowed = new Set(['app/src/renderer/project/tabs.ts']);
    const offenders: string[] = [];
    for (const [path, text] of rendererFiles) {
      const name = rel(path);
      if (name.includes('.test.')) continue;
      if (allowed.has(name) || name.includes('/project/health/')) continue;
      // **Not keyed on a variable name.** A first attempt matched `health.state ===` and a
      // §35-shaped consumer that called its parameter `summary` walked straight through it. The
      // two tests below are about the *vocabulary*: `suppressed` and `frozen` are `HealthState`'s
      // alone in this tree, and a file that imports a health type and branches on a `.state` is
      // branching on this one.
      const importsHealth = /HealthState|HealthSummary|HealthReading/u.test(text);
      for (const [i, line] of text.split('\n').entries()) {
        const code = line.trim();
        if (code.startsWith('//') || code.startsWith('*')) continue;
        if (/['"`](suppressed|frozen)['"`]/u.test(code)) {
          offenders.push(`${name}:${i + 1}`);
          continue;
        }
        if (importsHealth && /\.state\s*[=!]==/u.test(code)) {
          offenders.push(`${name}:${i + 1}`);
          continue;
        }
        // Reference exclusion re-derived beside the reading is the same defect one field along.
        if (/isReference/u.test(code) && /health/iu.test(code)) {
          offenders.push(`${name}:${i + 1}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});

describe('§30.13 audit 3 — the staleness sites', () => {
  /**
   * **R125.** `WORKTREE_STALE_AFTER_SECS = 900` stays at exactly one site,
   * `app/src/renderer/derive/observation.ts`, whose own comment says *"§6 … names no number. This
   * is that number."* §6 gains a **pointer**, not a value, and the health reading uses this
   * constant and introduces no second one.
   *
   * §32's 30-day clean-result expiry is a different quantity with its own name and owner, and
   * `REMOTE_STALE_AFTER_SECS = 21_600` sits under the same condition and is §21's.
   */
  it('AC-P3-30-15 900 appears as a staleness threshold at exactly one site', () => {
    // **Declaration sites, not mentions.** A test asserting the constant is not a second copy of
    // it, and the criterion is about where the value may be written down.
    const scanned = [...rendererFiles, ...coreFiles].filter(
      ([path]) => !rel(path).includes('.test.'),
    );
    const sites: string[] = [];
    for (const [path, text] of scanned) {
      for (const [i, line] of text.split('\n').entries()) {
        const code = line.trim();
        if (code.startsWith('//') || code.startsWith('*') || code.startsWith('///')) continue;
        if (!/\b900\b/u.test(code)) continue;
        // A staleness-shaped context, matched on **whole words**: `STAGE_FLOOR_MS = 900` and
        // `language_bytes` both spell `age` inside a longer word, and a substring test read both
        // as staleness thresholds. `_` is a word character to a regex, so an identifier is split
        // on it first — otherwise `WORKTREE_STALE_AFTER_SECS` is one word and matches nothing.
        const words = code.replace(/_/gu, ' ');
        if (!/\b(stale|staleness|age|ages|older|fresh|freshness|threshold)\b/iu.test(words)) {
          continue;
        }
        sites.push(`${rel(path)}:${i + 1}`);
      }
    }
    // eslint-disable-next-line no-console
    console.log(
      `AC-P3-30-15 scanned ${scanned.length} file(s); staleness sites naming 900: ${JSON.stringify(sites)}`,
    );
    expect(scanned.length).toBeGreaterThan(0);
    expect(sites.map((s) => s.split(':')[0])).toEqual(['app/src/renderer/derive/observation.ts']);
  });

  it('the health tab imports the constant by name and never a second literal', () => {
    const tab: string | null = readScannedFile(
      join(appDir, 'src/renderer/project/health/HealthTab.tsx'),
    );
    expect(tab).not.toBeNull();
    expect(tab).toContain('WORKTREE_STALE_AFTER_SECS');
    expect(tab).not.toMatch(/\b900\b/u);
  });
});

describe('§30.13 audit 4 — the word ban, scoped to renderings', () => {
  /**
   * **R130/F9.** §30.4's ban is about *rendering an absence as the presence of its opposite*, and
   * a wire enum is not a rendering. The words appear in **no rendered string, accessible name or
   * CSS class** in this feature; **a wire value named `clean` is permitted and is never rendered
   * as that word** — renaming §32's variant would be worse, because `clean` is the source's
   * concept and §32.13 already forbids re-spelling a third party's vocabulary.
   *
   * `app/src/renderer/derive/observation.ts` already holds the same ban for §6 and is the shape
   * this copies.
   */
  it('AC-P3-30-2 the words clean healthy none and all reach no rendered string in this feature', () => {
    const feature = rendererFiles.filter(([path]) => rel(path).includes('/project/health/'));
    // eslint-disable-next-line no-console
    console.log(`the word ban scanned ${feature.length} file(s) under project/health/`);
    expect(feature.length).toBeGreaterThan(0);

    const offenders: string[] = [];
    for (const [path, text] of feature) {
      if (rel(path).includes('.test.')) continue;
      // String literals and class names only — the audit reads renderings, never the schema.
      for (const [i, line] of text.split('\n').entries()) {
        const code = line.trim();
        if (code.startsWith('//') || code.startsWith('*') || code.startsWith('///')) continue;
        for (const literal of code.match(/'[^']*'|"[^"]*"|`[^`]*`/gu) ?? []) {
          if (/\b(clean|healthy|none|all)\b/iu.test(literal)) {
            offenders.push(`${rel(path)}:${i + 1} ${literal}`);
          }
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});

describe('§29.8 the in-context ask is not a modal and not a first-run row', () => {
  /**
   * **R137.** §29.8's three rendered sites moved together in p3-29's change, and a fourth hand on
   * one promise is how a promise drifts. This keeps **criterion 12 unmoved mechanically** rather
   * than by memory: `AC-P3-29-20` asserts the copy, and this asserts that the grant control never
   * reaches the first-run surface at all.
   */
  it('the grant control reaches no first-run surface', () => {
    const firstRun = rendererFiles.filter(([path]) => rel(path).includes('/renderer/firstrun/'));
    // eslint-disable-next-line no-console
    console.log(`the first-run guard scanned ${firstRun.length} file(s)`);
    expect(firstRun.length).toBeGreaterThan(0);

    for (const [path, text] of firstRun) {
      expect(text, rel(path)).not.toMatch(/GrantAsk|project\/health/u);
    }
  });
});

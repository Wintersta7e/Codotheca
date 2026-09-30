/**
 * **AC-P4-48-5, the ledger row** (§48.3 row 19, R158): consent row 3 names only the kinds of
 * ledger row a production writer stores. Revivals are derived and never written (§38.15), so the
 * row never names one.
 *
 * The written set is read from the core's source: the literal `kind` of every `INSERT INTO
 * xp_events` outside a test module. A restore that copies exported rows back names no literal
 * kind and adds nothing — it can only write back what a production writer wrote first.
 *
 * It lives in the node project and walks with `node:fs`: the same scan as an `import.meta.glob`
 * in the dom project raw-loads the whole core and times out under the parallel run
 * (`app/test/decayScope.test.ts` records the measurement for the renderer tree).
 */
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, it } from 'vitest';
import { withoutComments } from '../../scripts/lib/without-comments.mjs';
import { required } from '../src/shared/required';
import { CONSENT_ROWS } from '../src/renderer/firstrun/copy';

const CORE_SRC = fileURLToPath(new URL('../../core/src/', import.meta.url));

/** How the row's words name a ledger kind. */
const KIND_WORDS: readonly (readonly [string, RegExp])[] = [
  ['commit_day', /\bcommit-days?\b|\bdays you commit\b/i],
  ['release', /\breleases?\b/i],
  ['language_first', /\blanguages?\b/i],
  ['revival', /\breviv(?:al|als|e|es|ed)\b/i],
  ['focus', /\bfocus\b/i],
  ['debt_day', /\bdebt\b/i],
];

function rustFiles(dir: string, out: string[]): string[] {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) rustFiles(path, out);
    else if (entry.name.endsWith('.rs')) out.push(path);
  }
  return out;
}

/** Every literal `kind` a production `INSERT INTO xp_events` writes, with where it is written. */
function writtenKinds(): { files: number; kinds: Map<string, string[]> } {
  const kinds = new Map<string, string[]>();
  const files = rustFiles(CORE_SRC, []);
  for (const file of files) {
    const code = withoutComments(readFileSync(file, 'utf8'));
    const cut = code.indexOf('#[cfg(test)]');
    const production = cut === -1 ? code : code.slice(0, cut);
    for (const m of production.matchAll(
      /INSERT INTO xp_events\s*\(([^)]*)\)\s*VALUES\s*\(([^)]*)\)/g,
    )) {
      const columns = required(m[1], 'the column list')
        .split(',')
        .map((c) => c.trim());
      const values = required(m[2], 'the value list')
        .split(',')
        .map((v) => v.trim());
      const where = relative(CORE_SRC, file);
      expect(values.length, `${where}: columns and values do not pair`).toBe(columns.length);
      const kind = /^'([a-z_]+)'$/.exec(values[columns.indexOf('kind')] ?? '');
      if (kind === null) continue;
      const literal = required(kind[1], 'the kind literal');
      kinds.set(literal, [...(kinds.get(literal) ?? []), where]);
    }
  }
  return { files: files.length, kinds };
}

it('AC-P4-48-5 consent row 3 names only kinds a production writer stores', () => {
  const { files, kinds } = writtenKinds();
  process.stderr.write(
    `consentLedgerRow: ${String(files)} core files scanned; kinds written: ${[...kinds]
      .map(([kind, sites]) => `${kind} (${sites.join(', ')})`)
      .join('; ')}\n`,
  );
  expect(kinds.size, 'no production writer of xp_events was found').toBeGreaterThan(0);

  const row = required(CONSENT_ROWS[2], 'consent row 3');
  const text = `${row.body} ${row.note}`;
  const named = KIND_WORDS.filter(([, words]) => words.test(text)).map(([kind]) => kind);
  expect(named.length, `consent row 3 names no ledger kind: "${row.body}"`).toBeGreaterThan(0);
  expect(named, 'a revival is derived, never written').not.toContain('revival');
  for (const kind of named) {
    expect(kinds.has(kind), `consent row 3 names ${kind}, which no production writer stores`).toBe(
      true,
    );
  }
});

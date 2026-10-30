/**
 * **AC-P4-48-5, the ledger row** (§48.3 row 19, R158): consent row 3 names only the kinds of
 * ledger row a production writer stores. Revivals are derived and never written (§38.15), so the
 * row never names one.
 *
 * The written set is read from the core's source: the literal `kind` of every `INSERT INTO
 * xp_events` outside a test module, and the `kind` of every `GitRow` the history builds, which
 * J4's reconcile binds into its insert. A restore that copies exported rows back names no literal
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

/**
 * The text between the `(` at `open` and the `)` that balances it. A value list nests calls
 * (`json_set(?6, '$.silent', json('true'))`), so its first `)` is not its end; quoted SQL text
 * is skipped, so a `(` or `,` inside a string literal counts for nothing.
 */
function balanced(code: string, open: number): { inner: string; end: number } {
  let depth = 0;
  let quoted = false;
  for (let i = open; i < code.length; i += 1) {
    const c = code.charAt(i);
    if (c === "'") quoted = !quoted;
    else if (!quoted && c === '(') depth += 1;
    else if (!quoted && c === ')') {
      depth -= 1;
      if (depth === 0) return { inner: code.slice(open + 1, i), end: i + 1 };
    }
  }
  throw new Error(`unbalanced parentheses from offset ${String(open)}`);
}

/** The list's items, split on the commas outside every nested call and quoted text. */
function topLevel(list: string): string[] {
  const items: string[] = [];
  let depth = 0;
  let quoted = false;
  let start = 0;
  for (let i = 0; i < list.length; i += 1) {
    const c = list.charAt(i);
    if (c === "'") quoted = !quoted;
    else if (!quoted && c === '(') depth += 1;
    else if (!quoted && c === ')') depth -= 1;
    else if (!quoted && depth === 0 && c === ',') {
      items.push(list.slice(start, i).trim());
      start = i + 1;
    }
  }
  items.push(list.slice(start).trim());
  return items;
}

/** The literal `kind` of every `INSERT INTO xp_events … VALUES` in `code`, null where it is bound. */
function kindsIn(code: string, where: string): (string | null)[] {
  const kinds: (string | null)[] = [];
  for (const m of code.matchAll(/INSERT INTO xp_events\s*\(/g)) {
    const columns = balanced(code, m.index + m[0].length - 1);
    const lead = /^\s*VALUES\s*\(/.exec(code.slice(columns.end));
    if (lead === null) continue;
    const values = topLevel(balanced(code, columns.end + lead[0].length - 1).inner);
    const names = topLevel(columns.inner);
    expect(values.length, `${where}: columns and values do not pair`).toBe(names.length);
    const kind = /^'([a-z_]+)'$/.exec(values[names.indexOf('kind')] ?? '');
    kinds.push(kind === null ? null : required(kind[1], 'the kind literal'));
  }
  return kinds;
}

/** Every literal `kind` a production `INSERT INTO xp_events` writes, with where it is written. */
function writtenKinds(): { files: number; inserts: number; kinds: Map<string, string[]> } {
  const kinds = new Map<string, string[]>();
  const files = rustFiles(CORE_SRC, []);
  let inserts = 0;
  for (const file of files) {
    const code = withoutComments(readFileSync(file, 'utf8'));
    const cut = code.indexOf('#[cfg(test)]');
    const production = cut === -1 ? code : code.slice(0, cut);
    const where = relative(CORE_SRC, file);
    for (const literal of kindsIn(production, where)) {
      inserts += 1;
      if (literal !== null) kinds.set(literal, [...(kinds.get(literal) ?? []), where]);
    }
    // The git rows and the retained rows reach their one insert with the kind bound, not inline.
    for (const m of production.matchAll(
      /\b(?:GitRow|RetainedRow)\s*\{[^}]*?\bkind:\s*"([a-z_]+)"/g,
    )) {
      const literal = required(m[1], 'the row kind');
      kinds.set(literal, [...(kinds.get(literal) ?? []), where]);
    }
  }
  return { files: files.length, inserts, kinds };
}

it('reads a value list that nests calls and quotes a comma', () => {
  const planted = `"INSERT INTO xp_events (meta, ts, kind)
     VALUES (json_set(?1, '$.a,b', json('true')), ?2, 'planted_kind')"`;
  expect(kindsIn(planted, 'planted')).toEqual(['planted_kind']);
});

it('AC-P4-48-5 consent row 3 names only kinds a production writer stores', () => {
  const { files, inserts, kinds } = writtenKinds();
  process.stderr.write(
    `consentLedgerRow: ${String(files)} core files scanned, ${String(inserts)} inserts found; kinds written: ${[
      ...kinds,
    ]
      .map(([kind, sites]) => `${kind} (${sites.join(', ')})`)
      .join('; ')}\n`,
  );
  expect(inserts, 'no production insert into xp_events was found').toBeGreaterThan(0);
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

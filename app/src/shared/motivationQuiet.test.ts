/**
 * §43.11: quiet mode's closed list has **one owner**. The drawer, the renderer gate and the shell
 * read it from `motivationQuiet.ts`; a second spelling of an entry anywhere under `app/src` is a
 * second list that can drift from the first, so each entry's label must occur as a quoted literal
 * in exactly that one file.
 */
import { readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, test } from 'vitest';

import { readScannedFile } from '../../../scripts/lib/read-scanned.mjs';
import { DATA_MOTIVATION, QUIET_LIST, quietNote, type QuietEntryId } from './motivationQuiet';

const REPO = fileURLToPath(new URL('../../..', import.meta.url));
const SRC = join(REPO, 'app/src');
const OWNER = 'app/src/shared/motivationQuiet.ts';

function sourcesUnder(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...sourcesUnder(full));
    else if (/\.tsx?$/u.test(entry.name)) out.push(full);
  }
  return out;
}

/** The label written as a string literal in any of the three quote forms. */
function quoted(text: string, label: string): boolean {
  return ["'", '"', '`'].some((q) => text.includes(`${q}${label}${q}`));
}

test("AC-P4-43-17 the list's entries occur as literals in exactly one module", () => {
  const files: [string, string][] = [];
  for (const path of sourcesUnder(SRC)) {
    const text = readScannedFile(path);
    // A file another test removed mid-walk carries nothing and is not counted.
    if (text === null) continue;
    files.push([relative(REPO, path).replaceAll('\\', '/'), text]);
  }
  console.warn(`files scanned under app/src: ${String(files.length)}`);
  expect(files.length, 'the walk scanned no file').toBeGreaterThan(0);
  expect(QUIET_LIST.length).toBeGreaterThan(0);

  for (const entry of QUIET_LIST) {
    const holders = files.filter(([, text]) => quoted(text, entry.label)).map(([path]) => path);
    expect(holders, `${entry.id}: ${entry.label}`).toEqual([OWNER]);
  }
});

describe('the closed list of quiet mode', () => {
  it('holds one entry per hidden row of §43.11, each once', () => {
    const ids: QuietEntryId[] = QUIET_LIST.map((entry) => entry.id);
    expect(ids).toEqual([
      'level',
      'xp',
      'rings',
      'rhythm',
      'badges',
      'line',
      'identityLevelClause',
    ]);
    expect(new Set(QUIET_LIST.map((entry) => entry.label)).size).toBe(QUIET_LIST.length);
  });

  it('names the DOM attribute every covered element carries', () => {
    expect(DATA_MOTIVATION).toBe('data-motivation');
  });

  it('builds the drawer note from the entries, in their order, and from nothing else', () => {
    const note = quietNote();
    let from = 0;
    for (const entry of QUIET_LIST) {
      const at = note.indexOf(entry.label, from);
      expect(at, entry.label).toBeGreaterThanOrEqual(from);
      from = at + entry.label.length;
    }
    // Every character of the note is a label or the separator between two: nothing restated.
    expect(note).toBe(QUIET_LIST.map((entry) => entry.label).join(' · '));
  });
});

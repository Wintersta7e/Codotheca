/**
 * §40.1: a badge never notifies. The shell is the one place an OS notification is posted, so the
 * files that construct one — found by content, so the test stays true wherever the call site
 * moves — must take no badge input: none names a schema type whose name starts with `Badge`, and
 * the shell subscribes to no `badges` topic.
 */
import { readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from 'vitest';

import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';

const REPO = fileURLToPath(new URL('../..', import.meta.url));
const MAIN = join(REPO, 'app/src/main');
const CALL = 'new Notification(';

function sourcesUnder(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...sourcesUnder(full));
    else if (/\.tsx?$/u.test(entry.name) && !/\.test\.tsx?$/u.test(entry.name)) out.push(full);
  }
  return out;
}

function badgeTypes(): string[] {
  const text = readScannedFile(join(REPO, 'protocol/schema/protocol.json'));
  expect(text ?? '', 'protocol.json read as empty').not.toBe('');
  const doc: unknown = JSON.parse(text ?? '{}');
  const types =
    typeof doc === 'object' && doc !== null && 'types' in doc && typeof doc.types === 'object'
      ? doc.types
      : null;
  return Object.keys(types ?? {}).filter((name) => name.startsWith('Badge'));
}

test('ac_p4_40_5_the_notification_call_site_takes_no_badge_input', () => {
  const posting: [string, string][] = [];
  for (const file of sourcesUnder(MAIN)) {
    const text = readScannedFile(file);
    if (text === null) continue;
    if (text.includes(CALL)) posting.push([relative(REPO, file), text]);
  }
  console.warn(
    `files constructing a Notification: ${String(posting.length)}`,
    posting.map(([f]) => f),
  );
  expect(posting.length, 'no file under app/src/main constructs a Notification').toBeGreaterThan(0);

  const types = badgeTypes();
  console.warn(`schema Badge types: ${String(types.length)}`, types);
  expect(types.length, 'the schema declares no Badge type').toBeGreaterThan(0);

  const named = posting.flatMap(([file, text]) =>
    types
      .filter((type) => new RegExp(`\\b${type}\\b`, 'u').test(text))
      .map((type) => `${file}: ${type}`),
  );
  expect(named, 'a notification call site takes a badge type').toEqual([]);

  const index = readScannedFile(join(MAIN, 'index.ts')) ?? '';
  const topics = /const\s+TOPICS\s*:\s*Topic\[\]\s*=\s*\[([^\]]*)\]/u.exec(index)?.[1];
  expect(topics, 'the shell declares no TOPICS array').toBeDefined();
  const names = [...(topics ?? '').matchAll(/'([^']+)'/gu)].map((m) => m[1]);
  console.warn(`shell topics: ${String(names.length)}`, names);
  expect(names.length, 'the TOPICS array names nothing').toBeGreaterThan(0);
  expect(names, 'the shell subscribes to badges').not.toContain('badges');
});

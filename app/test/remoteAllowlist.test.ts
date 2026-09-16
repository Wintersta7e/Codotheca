import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { REMOTE_HOST_ALLOWLIST } from '../src/main/dialogs/externalLink';

/**
 * §25.2's host allowlist is a cross-language mirror, and it decides whether a URL is handed to
 * the operating system. R24's remedy applies in both halves:
 *
 * - **The base half** is one literal in Rust and one in TypeScript, and this file reads the Rust
 *   one as text so the two cannot drift.
 * - **The Enterprise half has one source and no literal at all.** The core reads `account.host`
 *   out of SQLite; the shell reads the same column through `accounts.list`. One value with two
 *   readers is fine — one value with two *sources* is R12, on the value that decides whether a
 *   link opens.
 *
 * The scan is over **string literals**, not over the whole source: a hostname in a comment
 * reaches no allowlist, and a gate that fired on prose would be turned off within a week.
 */
const REPO = fileURLToPath(new URL('../..', import.meta.url));
const CORE_WEBURL = join(REPO, 'core/src/remote/weburl.rs');
const SHELL_OPENER = join(REPO, 'app/src/main/dialogs/externalLink.ts');

function coreSource(): string {
  const source = readFileSync(CORE_WEBURL, 'utf8');
  // The guard `app/test/roastScope.test.ts:70-72` states for a scanning gate: a read that
  // returned nothing makes every assertion below vacuous.
  expect(source.length, 'the core allowlist source was not read').toBeGreaterThan(200);
  return source;
}

function allowlistBaseFromRust(): string[] {
  const block = /pub const ALLOWLIST_BASE: &\[&str\] = &\[([^\]]*)\];/u.exec(coreSource());
  const body = block?.[1];
  if (body === undefined)
    throw new Error('ALLOWLIST_BASE is declared in core/src/remote/weburl.rs');
  return [...body.matchAll(/"([^"]+)"/gu)].map((m) => m[1] ?? '');
}

/** Every declared command name, so a literal that is one is not mistaken for a host. */
function commandNames(): Set<string> {
  const doc: unknown = JSON.parse(
    readFileSync(join(REPO, 'protocol/schema/protocol.json'), 'utf8'),
  );
  const commands =
    typeof doc === 'object' && doc !== null && 'commands' in doc ? doc.commands : null;
  if (!Array.isArray(commands) || commands.length === 0) {
    throw new Error('protocol.json must declare a non-empty command array');
  }
  return new Set(
    commands.map((c: unknown) =>
      typeof c === 'object' && c !== null && 'name' in c ? String(c.name) : '',
    ),
  );
}

/** Whole single- or double-quoted literals that read as a hostname. */
function hostLiteralsIn(source: string): string[] {
  const commands = commandNames();
  const found: string[] = [];
  for (const match of source.matchAll(/['"]([^'"\n]+)['"]/gu)) {
    const literal = match[1] ?? '';
    if (!/^[a-z0-9-]+(?:\.[a-z0-9-]+)+$/u.test(literal)) continue;
    if (commands.has(literal)) continue;
    found.push(literal);
  }
  return found;
}

describe('§25.2: the host allowlist is one value with two readers', () => {
  it('the shell base list equals the core one, character for character', () => {
    const rust = allowlistBaseFromRust();
    expect(rust.length, 'ALLOWLIST_BASE parsed to nothing').toBeGreaterThan(0);
    expect([...REMOTE_HOST_ALLOWLIST]).toEqual(rust);
  });

  it('the shell holds no host literal beyond that base list', () => {
    const source = readFileSync(SHELL_OPENER, 'utf8');
    expect(source.length, 'the shell opener source was not read').toBeGreaterThan(200);
    const unexpected = hostLiteralsIn(source).filter(
      (host) => !REMOTE_HOST_ALLOWLIST.includes(host),
    );
    expect(unexpected, 'a host literal in the shell is a second source for the allowlist').toEqual(
      [],
    );
  });

  it('the core reads its Enterprise input from the account row and from no other column', () => {
    const source = coreSource();
    expect(source).toMatch(/SELECT DISTINCT host FROM account/u);
    // The shell reads the same column, through the command that serialises it. If these two
    // names ever differ, the fix is one of them moving — never a translation layer, which is
    // how one value becomes two.
    const shell = readFileSync(SHELL_OPENER, 'utf8');
    expect(shell).toMatch(/accounts\.list/u);
    expect(shell).toMatch(/account\.host/u);
  });
});

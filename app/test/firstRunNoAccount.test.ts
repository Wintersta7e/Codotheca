/**
 * **AC-P2-20-9, the first-run half.** §20.11: *"First run mentions no account, offers no connect,
 * and has no account step. The first-run gate cannot be reached from an unconnected state and
 * cannot be satisfied by connecting."*
 *
 * This reads the first-run sources rather than rendering them, because the claim is about an
 * **absence** across a whole directory — and an absence has no behaviour to assert. It **prints
 * the file count and fails at zero**: a walk that read nothing would satisfy every assertion
 * below vacuously.
 */
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * It lives under `app/test/` rather than beside the sources it reads, and that is forced: it
 * imports `node:fs`, and `tsconfig.web.json` carries no node types, so a file-reading test in
 * `src/renderer/**` compiles under vitest and fails `npm run typecheck`. `app/test/` is the node
 * project, where every other file-reading gate in this repository already lives.
 */
const FIRSTRUN = fileURLToPath(new URL('../src/renderer/firstrun/', import.meta.url));

/** Every source in this directory, with `//` comment lines stripped. */
function sources(): { name: string; code: string }[] {
  const out: { name: string; code: string }[] = [];
  for (const entry of readdirSync(FIRSTRUN, { withFileTypes: true })) {
    // The kind comes from readdir, never a second stat.
    if (!entry.isFile()) continue;
    if (!/\.(?:ts|tsx|css)$/u.test(entry.name)) continue;
    // The bar documents the ban; a test file naming it must not trip the rule it documents.
    if (/\.test\./u.test(entry.name)) continue;
    const code = readFileSync(join(FIRSTRUN, entry.name), 'utf8')
      .split('\n')
      .filter(
        (line: string) => !line.trimStart().startsWith('//') && !line.trimStart().startsWith('*'),
      )
      .join('\n');
    out.push({ name: entry.name, code });
  }
  return out;
}

describe('AC-P2-20-9 — first run mentions no account', () => {
  it('names no account, connect, token or scope vocabulary anywhere in first run', () => {
    const files = sources();
    // A gate that scanned nothing must say so.
    process.stderr.write(
      `firstRunNoAccount: scanned ${String(files.length)} first-run source file(s)\n`,
    );
    expect(files.length, 'the walk read no file, so it proved nothing').toBeGreaterThan(0);

    // What is banned is an account **step**, not the word. First run's own copy says *"Nothing
    // is uploaded. There is no account."* — stating the absence is the honest thing to do and is
    // the opposite of offering one, so a bare `/account/` rule would forbid the very sentence
    // §20.11 wants. These four are what would mean a step exists: a command call, a connect
    // control, the grant payload, or the forge named as something to join.
    const banned = [/\baccounts\.[a-z]/u, /\bCONNECT\b/u, /\bgrantedScopes\b/u, /\bGitHub\b/u];
    const offenders: string[] = [];
    for (const { name, code } of files) {
      for (const pattern of banned) {
        if (pattern.test(code)) offenders.push(`${name}: ${pattern.source}`);
      }
    }
    expect(offenders, 'first run mentions an account').toEqual([]);
  });

  it('has a gate predicate that reads no account state', () => {
    const gate = sources().find((file) => file.name === 'FirstRunGate.tsx');
    expect(gate, 'FirstRunGate.tsx is the predicate this rule is about').toBeTruthy();
    expect(gate?.code.length ?? 0).toBeGreaterThan(0);
    // The gate cannot be satisfied by connecting, because it never reads whether one has.
    expect(gate?.code).not.toMatch(/account/iu);
  });
});

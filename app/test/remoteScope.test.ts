import { readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';

/**
 * Two gates over one tree.
 *
 * **The caller gate (§25.8).** *"The shell is its only caller in phase 1."* `remote.webUrl` is
 * not privileged — `protocol/lib/schema.mjs` throws on a privileged command that carries no
 * `Bytes` — so the bridge will answer it for the renderer, and nothing about the capability
 * enforces that sentence. This does. An earlier revision of the plan asserted that
 * `isRendererCallable` was not widened; that was false, and nothing checked it, which is the
 * shape of a bar written past its defect.
 *
 * Both halves read every file through `scripts/lib/read-scanned.mjs`'s contract: other gates
 * plant probe files inside `app/src/renderer/` while vitest runs the node project in parallel,
 * so a walked path can be gone by the time it is read, and an unguarded read throws a raw ENOENT
 * that takes the whole suite red. A vanished file is skipped **before** it is counted, or the
 * "scanned nothing" guard stops meaning what it says.
 */
const appDir = fileURLToPath(new URL('..', import.meta.url));
const srcDir = join(appDir, 'src');

function walk(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    let stat;
    try {
      stat = statSync(path);
    } catch (error: unknown) {
      if (isMissing(error)) continue;
      throw error;
    }
    if (stat.isDirectory()) walk(path, out);
    else if (/\.tsx?$/u.test(entry) && !/\.test\.tsx?$/u.test(entry)) out.push(path);
  }
  return out;
}

function isMissing(error: unknown): boolean {
  return typeof error === 'object' && error !== null && 'code' in error && error.code === 'ENOENT';
}

const files: readonly (readonly [string, string])[] = walk(srcDir).flatMap((path) => {
  const text = readScannedFile(path);
  return text === null ? [] : [[relative(appDir, path).replace(/\\/gu, '/'), text] as const];
});

/** A file's source with its comment lines removed. */
function codeOf(text: string): string {
  return text
    .split('\n')
    .filter((line) => {
      const t = line.trimStart();
      return !t.startsWith('//') && !t.startsWith('*') && !t.startsWith('/*');
    })
    .join('\n');
}

function matching(pattern: RegExp): string[] {
  return files
    .filter(([, text]) => pattern.test(text))
    .map(([path]) => path)
    .sort();
}

describe('§25.8: the shell is `remote.webUrl`s only caller', () => {
  it('scanned a real tree, or every assertion below is vacuous', () => {
    process.stderr.write(
      `remoteScope: caller gate scanned ${String(files.length)} source file(s)\n`,
    );
    expect(files.length, 'the caller gate scanned nothing, so it proved nothing').toBeGreaterThan(
      40,
    );
  });

  /**
   * Four files may name it and the list is exact, not a bound. Three of them are tables rather
   * than callers: `src/generated/protocol.ts` is the contract and declares every command,
   * `src/main/core/idempotence.ts` classifies every command as a read or a write, and
   * `src/main/index.ts` is `KNOWN_COMMANDS`, the list the bridge will accept. The opener is the
   * only thing that issues it.
   */
  it('names the command in exactly four files, and only one of them calls it', () => {
    expect(matching(/remote\.webUrl/u)).toEqual([
      'src/generated/protocol.ts',
      'src/main/core/idempotence.ts',
      'src/main/dialogs/externalLink.ts',
      'src/main/index.ts',
    ]);
  });

  it('names it in no renderer module at all', () => {
    const renderer = files.filter(([path]) => path.startsWith('src/renderer/'));
    expect(renderer.length, 'the renderer half of the walk found nothing').toBeGreaterThan(20);
    expect(renderer.filter(([, text]) => text.includes('remote.webUrl')).map(([p]) => p)).toEqual(
      [],
    );
  });

  it('calls shell.openExternal from the main process only', () => {
    expect(matching(/openExternal/u).filter((p) => p.startsWith('src/renderer/'))).toEqual([]);
  });
});

/**
 * **§25.4's renderer gate (AC-P2-25-7's second half).** *"Phase 2 displays the CI record. It may
 * not judge it."* The cheap half of that is keeping the forge types out of every surface that
 * would then acquire a reason to score them, and it is automated rather than remembered.
 *
 * **The allowed set is an array, not a bound, and it is narrower than §25.4 in one direction and
 * wider in another. Both are deliberate.**
 *
 * - **Narrower**: §25.4 admits *"the identity line's visibility segment"*, and `Identity.tsx`
 *   takes `RemoteVisibility | null` rather than the whole struct — so it imports neither
 *   `RemoteFacts` nor a CI type. `ReadmePanel.tsx` takes `readonly string[]` for the same
 *   reason. *"Only by X"* is satisfied by a subset, so dropping them is stricter.
 * - **Wider**: `shelf/Peek.tsx` renders §25.3a's positive half **from the same producer**, at
 *   Peek's size. §25.4 was written before that producer was placed, and refusing the import
 *   would force a copy — the defect this gate exists to catch.
 * - `project/testFixtures.ts` builds the shape every test on this page uses. It renders
 *   nothing, and excluding it would push each test to hand-build a `RemoteFacts`, which is the
 *   drift one fixture exists to prevent.
 *
 * Record both in this comment, or a later reader narrows the list back and Peek grows its own
 * four-state logic.
 */
describe('§25.4: the forge types are imported only by the surfaces that render them', () => {
  const FORGE_TYPES = ['RemoteFacts', 'CiList', 'CiRun'] as const;

  const ALLOWED = [
    'src/renderer/project/remote/RemoteTab.tsx',
    'src/renderer/project/remote/ciCopy.ts',
    'src/renderer/project/remote/links.tsx',
    'src/renderer/project/remote/remoteBlocks.tsx',
    'src/renderer/project/testFixtures.ts',
    'src/renderer/shelf/Peek.tsx',
    'src/renderer/shelf/peekText.ts',
  ];

  it('scanned a real renderer tree, or every assertion below is vacuous', () => {
    const renderer = files.filter(([path]) => path.startsWith('src/renderer/'));
    process.stderr.write(
      `remoteScope: import gate scanned ${String(renderer.length)} renderer source file(s)\n`,
    );
    expect(renderer.length, 'the import gate scanned nothing').toBeGreaterThan(20);
  });

  it('AC-P2-25-7-renderer names them nowhere else under src/renderer', () => {
    // Comment lines are stripped first. *Grepping a declaration also matches prose about it* is
    // a recorded defect of this project, and it fired here immediately: `ReadmePanel.tsx` says
    // in a comment that it takes `readonly string[]` and **not** `RemoteFacts`, which is the
    // rule this gate enforces rather than a violation of it.
    const naming = files
      .filter(([path]) => path.startsWith('src/renderer/'))
      .filter(([, text]) =>
        FORGE_TYPES.some((type) => new RegExp(`\\b${type}\\b`, 'u').test(codeOf(text))),
      )
      .map(([path]) => path)
      .sort();
    // A naming set that matched nothing would satisfy the filter above while proving nothing,
    // so the tab's own module has to be in it.
    expect(naming).toContain('src/renderer/project/remote/RemoteTab.tsx');
    expect(naming.filter((path) => !ALLOWED.includes(path))).toEqual([]);
  });
});

/**
 * **AC-P2-25-23's import half.** §25.3a renders the tab's blocks *at Peek's size*, **from the
 * same producer** — it is not a second place they are computed.
 *
 * §25.4's importer gate cannot catch this on its own: it **admits** `Peek.tsx` deliberately, so
 * a copy of the four-state logic written inside that file passes it. The plan expected that gate
 * to name the file; it cannot, and this is the assertion that can — the same import shape
 * AC-P2-25-5 uses for `fetchClause`.
 */
describe('§25.3a Peek draws the blocks from the tab producer, not a copy', () => {
  it('AC-P2-25-23-producer imports RemoteBlock rather than declaring one', () => {
    const found = files.find(([path]) => path === 'src/renderer/shelf/Peek.tsx');
    expect(found, 'Peek.tsx was not scanned').toBeDefined();
    const source = found?.[1] ?? '';
    expect(source.length, 'the source was not read').toBeGreaterThan(200);
    expect(source).toMatch(
      /import \{[^}]*\bRemoteBlock\b[^}]*\} from '\.\.\/project\/remote\/remoteBlocks\.js'/u,
    );
    const code = codeOf(source);
    expect(code).not.toMatch(/function RemoteBlock\b/u);
    expect(code).not.toMatch(/const RemoteBlock\b/u);
    // The four-state vocabulary belongs to the producer. A copy here would have to name one.
    expect(code.includes("'not_observed'")).toBe(false);
    expect(code.includes("'no_account'")).toBe(false);
  });
});

/**
 * **AC-P2-25-8's source half.** *"A build capable of rendering `PRIVATE` while having no code
 * path that renders `PUBLIC` fails."* A rendered-output test alone cannot see that: a build with
 * no `PUBLIC` branch renders correctly on every `private` fixture, and the absence of the word is
 * exactly what makes its absence assert public.
 */
describe('§25.3 the identity line has a code path for each word', () => {
  it('AC-P2-25-8-source names both words, in code and not only in prose', () => {
    const found = files.find(([path]) => path === 'src/renderer/project/Identity.tsx');
    expect(found, 'Identity.tsx was not scanned').toBeDefined();
    const source = found?.[1] ?? '';
    expect(source.length, 'the source was not read').toBeGreaterThan(200);
    const code = source
      .split('\n')
      .filter((line) => !line.trimStart().startsWith('*') && !line.trimStart().startsWith('//'))
      .join('\n');
    expect(code).toContain("'PUBLIC'");
    expect(code).toContain("'PRIVATE'");
  });
});

/**
 * **AC-P2-25-5's import half.** §25.1's `BEHIND` is produced by the module §8.5.2 already uses,
 * *asserted by import and not by matching text*: a copy inlined into the tab would render the
 * same strings and pass every rendered-output assertion in this repository.
 */
describe('§25.1 BEHIND comes from the producer §8.5.2 already uses', () => {
  function sourceOf(path: string): string {
    const found = files.find(([name]) => name === path);
    expect(found, `${path} was not scanned`).toBeDefined();
    return found?.[1] ?? '';
  }

  it('AC-P2-25-5-import imports the fetch clause rather than re-writing it', () => {
    const source = sourceOf('src/renderer/project/remote/behindBlock.ts');
    expect(source.length, 'the source was not read').toBeGreaterThan(200);
    expect(source).toMatch(/import \{ fetchClause \} from '\.\.\/locations\/locationCopy'/u);
  });

  it('holds neither half of the clause as a literal of its own', () => {
    // Comment lines are stripped: the module explains the import, and the explanation names the
    // strings it must not own.
    const code = sourceOf('src/renderer/project/remote/behindBlock.ts')
      .split('\n')
      .filter((line) => !line.trimStart().startsWith('*') && !line.trimStart().startsWith('//'))
      .join('\n');
    expect(code.includes('no fetch recorded')).toBe(false);
    expect(code.includes('last fetch')).toBe(false);
  });
});

/**
 * **Every command the renderer calls must actually be callable from the renderer.**
 *
 * This is the bar that was missing, and its absence cost the whole Uninstall feature. Three
 * separate lists have to agree for a renderer call to reach the core — the renderer's own
 * `request('<name>')`, the shell's `KNOWN_COMMANDS`, and the core's router — and **nothing
 * compared the first against the second**. `locations.uninstallPreflight` was in the renderer and
 * in neither of the others, so a press produced a `PROTOCOL` refusal in 13 ms, the renderer
 * rendered the refusal as *nothing happened*, and the shell log named no error. Every unit test
 * passed, because every one of them mocks the request function and none crosses the door.
 *
 * `knownCommands.test.ts` compares the shell's list against the **schema**, which is why it was
 * green: the schema declared the command, the core declared it unowned, and the two lists were
 * consistent with each other and wrong about the product.
 *
 * The scan reads comment-stripped source: a command name in a docblock is prose about a call,
 * not a call.
 */
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';
import { withoutComments } from '../../scripts/lib/without-comments.mjs';
import { isRendererCallable } from '../src/main/core/bridge';

const REPO = fileURLToPath(new URL('../..', import.meta.url));
const RENDERER = join(REPO, 'app/src/renderer');

/**
 * `KNOWN_COMMANDS` read out of its own declaration rather than imported.
 *
 * Importing `app/src/main/index.ts` pulls in Electron, which is not available under vitest's node
 * project. The same technique `knownCommands.test.ts` uses, and for the same reason.
 */
function knownCommands(): readonly string[] {
  const source = readFileSync(join(REPO, 'app/src/main/index.ts'), 'utf8');
  const block = /const\s+KNOWN_COMMANDS:\s*readonly\s+CommandName\[\]\s*=\s*\[([\s\S]*?)\];/u.exec(
    source,
  );
  if (block?.[1] === undefined) throw new Error('KNOWN_COMMANDS is declared in main/index.ts');
  return [...withoutComments(block[1]).matchAll(/'([^']+)'/gu)].map((m) => m[1] ?? '');
}

/** Every `.ts`/`.tsx` under the renderer, comments blanked, read through the shared guard. */
function rendererSources(dir = RENDERER): { path: string; code: string }[] {
  const out: { path: string; code: string }[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...rendererSources(full));
      continue;
    }
    if (!/\.tsx?$/u.test(entry.name)) continue;
    const text = readScannedFile(full);
    // Skipped **before** it is counted: the zero-file guard below has to keep meaning what it
    // says while other gates plant probes in these directories.
    if (text === null) continue;
    out.push({ path: full.slice(RENDERER.length + 1), code: withoutComments(text) });
  }
  return out;
}

/**
 * Every command name the renderer passes to a `request(...)`, with the file that passes it.
 *
 * Matched on the **call**, not on the string: a command name in a type, a map key or a comment is
 * not a call, and `COMMAND_EFFECT`-shaped tables legitimately name every command there is.
 */
function calledCommands(): Map<string, string[]> {
  const found = new Map<string, string[]>();
  for (const { path, code } of rendererSources()) {
    for (const match of code.matchAll(/\brequest\(\s*'([a-zA-Z]+\.[a-zA-Z]+)'/gu)) {
      const name = match[1] ?? '';
      found.set(name, [...(found.get(name) ?? []), path]);
    }
  }
  return found;
}

describe('the renderer calls only commands that reach the core', () => {
  it('finds real calls to scan, and does not pass by finding none', () => {
    const called = calledCommands();
    // A scan that found nothing would make every assertion below vacuous — which is the exact
    // shape of gate this file exists because the product lacked.
    expect(called.size, 'the renderer scan found no request() call at all').toBeGreaterThan(10);
    // A known-good name, so a regex that silently stopped matching is caught here rather than by
    // reporting a clean run.
    expect([...called.keys()]).toContain('settings.get');
  });

  it('is offered every one of them by the shell', () => {
    const known = knownCommands();
    expect(known.length, 'KNOWN_COMMANDS read as empty').toBeGreaterThan(20);

    const unreachable: string[] = [];
    for (const [name, files] of calledCommands()) {
      if (!isRendererCallable(name, known)) unreachable.push(`${name} (${files.join(', ')})`);
    }

    expect(
      unreachable.sort(),
      'the renderer calls these and the bridge refuses them: either the shell must offer the ' +
        'command, or the renderer must reach it through a shell-owned channel instead',
    ).toEqual([]);
  });

  it('never names a privileged command, which travels its own channel', () => {
    const known = knownCommands();
    const privileged = [...calledCommands().keys()].filter(
      (name) => known.includes(name) && !isRendererCallable(name, known),
    );
    expect(
      privileged,
      'a privileged command is refused at the renderer door by design — the renderer names a ' +
        'channel instead',
    ).toEqual([]);
  });
});

/**
 * §24's shell- and renderer-side acceptance criteria, stated as the criteria state them.
 *
 * **Not a second copy of the unit suites.** `UninstallControl.test.tsx` proves the control
 * behaves; `bridge.test.ts` proves the guard refuses. A criterion is a sentence about the
 * product, and R46's finding was that the structure making something checkable gets built while
 * the check itself is assumed to be somebody's next step. Each test below is named for its
 * criterion and asserts the **whole** sentence — including the word `every`, which is the half a
 * per-command test has no reason to state.
 *
 * **Every scan below reads comment-stripped source**, through the destructive-token gate's own
 * `withoutComments`. A first draft did not, and both scans failed against this file's neighbours
 * for *describing* the rule they enforce — `uninstallCopy.ts` names `RECLAIM SPACE` to say it
 * ships nowhere, and `UninstallControl.tsx` says in a comment that there is no *uninstall
 * anyway*. A gate that cannot tell a rule from its statement is a gate nobody can document
 * around.
 */
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';
import { withoutComments } from '../../scripts/lib/without-comments.mjs';
import { isRendererCallable } from '../src/main/core/bridge';
import {
  IPC_INSTALL_CANCEL,
  IPC_INSTALL_START,
  IPC_RELOCATE,
  IPC_REQUEST,
  IPC_UNINSTALL,
} from '../src/shared/channels';

const REPO = fileURLToPath(new URL('../..', import.meta.url));
const APP_SRC = join(REPO, 'app/src');

interface Command {
  readonly name: string;
  readonly args?: Readonly<Record<string, string>>;
  readonly returns?: string;
  readonly privileged?: boolean;
  readonly idempotent?: boolean;
  readonly mutatesFilesystem?: boolean;
  readonly $comment?: string;
}

function schema(): { readonly commands: readonly Command[] } {
  const text = readFileSync(join(REPO, 'protocol/schema/protocol.json'), 'utf8');
  return JSON.parse(text) as { readonly commands: readonly Command[] };
}

/**
 * Every non-test `.ts`/`.tsx` file under `app/src`, comments blanked, read through
 * `readScannedFile`.
 *
 * Other gates plant probe files in these directories and vitest runs the node project in
 * parallel, so an unguarded read throws a raw ENOENT and takes the whole suite red. A vanished
 * file is skipped **before** it is counted — counting it would make the "scanned nothing" guard
 * in each test stop meaning what it says.
 */
function appCode(dir = APP_SRC): { path: string; code: string }[] {
  const out: { path: string; code: string }[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== 'generated') out.push(...appCode(path));
      continue;
    }
    if (!/\.tsx?$/u.test(entry.name) || /\.test\.tsx?$/u.test(entry.name)) continue;
    const text: string | null = readScannedFile(path);
    if (text === null) continue;
    out.push({
      path: path.slice(APP_SRC.length + 1).replaceAll('\\', '/'),
      code: withoutComments(text),
    });
  }
  return out;
}

// ---------------------------------------------------------------------------
// AC-P2-24-19 — every disk-mutating command, not one of them
// ---------------------------------------------------------------------------

describe('AC-P2-24-19: every disk-mutating command is off the renderer-callable surface', () => {
  /**
   * The criterion says **every**, and no single plan declares all three: p2-24 asserted the two
   * install commands and this plan asserts `locations.uninstall`. A suite asserting one third of
   * "every" and reporting the id closed is the defect this test exists to prevent, so the list is
   * derived from the schema rather than typed out — a fourth mutating command joins it by
   * existing.
   */
  const mutating = (): readonly Command[] => schema().commands.filter((c) => c.mutatesFilesystem);

  it('finds the three the schema declares, and derives them rather than naming them', () => {
    const names = mutating()
      .map((c) => c.name)
      .sort();
    expect(names).toEqual(['install.cancel', 'install.start', 'locations.uninstall']);
  });

  it('marks each one privileged', () => {
    for (const command of mutating()) {
      expect(command.privileged, `${command.name} mutates the filesystem`).toBe(true);
    }
  });

  it('refuses each one by name on IPC_REQUEST', () => {
    const known = schema().commands.map((c) => c.name);
    for (const command of mutating()) {
      expect(
        isRendererCallable(command.name, known),
        `${command.name} must not be reachable from the sandboxed renderer`,
      ).toBe(false);
    }
    // The guard is refusing these for being privileged, not because it refuses everything.
    expect(isRendererCallable('locations.uninstallPreflight', known)).toBe(true);
  });

  it('gives each one a shell-owned channel of its own', () => {
    const channels = [IPC_UNINSTALL, IPC_INSTALL_START, IPC_INSTALL_CANCEL];
    expect(new Set(channels).size, 'one channel per command, never one shared door').toBe(3);
    for (const channel of channels) {
      expect(channel).not.toBe(IPC_REQUEST);
      expect(channel.startsWith('codotheca:')).toBe(true);
    }
  });

  it('leaves the pre-flight unprivileged, because it writes nothing', () => {
    const preflight = schema().commands.find((c) => c.name === 'locations.uninstallPreflight');
    expect(preflight, 'the pre-flight is declared').toBeDefined();
    expect(preflight?.mutatesFilesystem).toBeUndefined();
    expect(preflight?.privileged).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// AC-P2-24-20 — relocate is untouched, and gains no call site
// ---------------------------------------------------------------------------

describe('AC-P2-24-20: locations.relocate is byte-identical to phase 1 and gains no call site', () => {
  /**
   * Read from the tree at the phase-1 schema tip, `cc13f496` — the last commit whose
   * `protocol.json` declared 42 commands — and pinned here as a literal rather than re-read from
   * git, so this test says what the values are instead of asking the reader to go and look.
   *
   * §24.6's rule is that `RELOCATE` is **not widened** to carry the removal. The way that rule
   * dies is somebody adding a `force` argument or flipping `privileged`, and the `$comment` is
   * included because it is where the reason lives: a comment that quietly grew a second clause is
   * the same change wearing different clothes.
   */
  const PHASE_1 = {
    name: 'locations.relocate',
    args: { locationId: 'LocationId', pathBytes: 'Bytes' },
    returns: 'LocationDetail',
    privileged: true,
    $comment:
      "§17: rewrites path_bytes in place and keeps scan_generation. It is the only phase-1 write to a location's path",
  } as const;

  it('declares exactly what phase 1 declared', () => {
    const relocate = schema().commands.find((c) => c.name === PHASE_1.name);
    expect(relocate, 'locations.relocate is still declared').toBeDefined();
    expect(relocate?.args).toEqual(PHASE_1.args);
    expect(relocate?.returns).toBe(PHASE_1.returns);
    expect(relocate?.privileged).toBe(PHASE_1.privileged);
    expect(relocate?.$comment).toBe(PHASE_1.$comment);
    expect(
      relocate?.mutatesFilesystem,
      'relocate rewrites a row; it moves no byte and must not claim to',
    ).toBeUndefined();
  });

  /**
   * **A call site, not a mention.** The first draft of this test also matched `relocate:` and
   * failed on two files that only *satisfy the interface* — `renderer/testing/deps.tsx` stubs it
   * to `cancelled` and `main/core/idempotence.ts` must name every command the schema declares.
   * Those are the type surface, not a path to the command, and counting them would have made the
   * criterion fail for a card fix that never touched relocate.
   *
   * The eight below were measured against `cc13f496` with this same matcher and are **the same
   * eight**. The count is printed: a gate whose passing run scans zero files is a failing gate.
   */
  it('gains no new call site', () => {
    const sources = appCode();
    expect(sources.length, 'the app scan read nothing').toBeGreaterThan(100);

    const invokes = [/['"]locations\.relocate['"]/u, /\bIPC_RELOCATE\b/u, /\.relocate\s*\(/u];
    const sites = sources
      .filter(({ code }) => invokes.some((pattern) => pattern.test(code)))
      .map(({ path }) => path)
      .sort();

    // eslint-disable-next-line no-console -- a gate that cannot say what it scanned proves nothing.
    console.log(
      `AC-P2-24-20: ${sources.length} app sources scanned, ${sites.length} relocate call sites`,
    );

    expect(sites, 'relocate is not widened and is not spread').toEqual([
      'main/core/idempotence.ts',
      'main/dialogs/relocate.ts',
      'main/index.ts',
      'preload/index.ts',
      'renderer/app/deps.ts',
      'renderer/project/deps.ts',
      'renderer/project/locations/LocationsPanel.tsx',
      'shared/channels.ts',
    ]);
  });

  it('is not the channel the removal travels', () => {
    expect(IPC_UNINSTALL).not.toBe(IPC_RELOCATE);
  });
});

// ---------------------------------------------------------------------------
// AC-P2-24-14 — the control, stated as the criterion states it
// ---------------------------------------------------------------------------

describe('AC-P2-24-14: no path past a non-safe disposition exists in the sources', () => {
  /**
   * `UninstallControl.test.tsx` renders every one of the fourteen blockers and asks the **DOM**
   * whether anything at all could reach the removal — that is the behavioural half, and it is
   * where the criterion's *"disabled with its reasons named"* and *"`unknown` renders distinctly
   * from `blocked`"* are proved.
   *
   * This is the half a render test cannot reach: a confirmation path, an override or a bulk
   * selection living in a file the control does not mount would never appear in its DOM. The
   * scan is over the whole app, so a second surface offering the removal fails here.
   */
  it('finds no override, confirmation-through or bulk wording anywhere in the app', () => {
    const sources = appCode();
    expect(sources.length, 'the app scan read nothing').toBeGreaterThan(100);

    const banned = [
      /uninstall\s+anyway/iu,
      /remove\s+anyway/iu,
      /force\s+uninstall/iu,
      /\bselectAll\b/u,
      /\bmultiSelect\b/u,
      /type\s+the\s+(number|count)/iu,
      /\bRECLAIM\s+SPACE\b/u,
    ];

    const hits: string[] = [];
    for (const { path, code } of sources) {
      for (const pattern of banned) {
        if (pattern.test(code)) hits.push(`${path} :: ${String(pattern)}`);
      }
    }

    // eslint-disable-next-line no-console -- print the count so a zero-file scan cannot read green.
    console.log(`AC-P2-24-14: ${sources.length} app sources scanned, ${hits.length} override hits`);
    expect(hits, 'phase 2 ships no override of any kind').toEqual([]);
  });

  /**
   * **The quoted command name, not the prefix.** A first form of this test used
   * `code.includes('locations.uninstall')`, which also matches
   * `'locations.uninstallPreflight'` — a command that is *deliberately* renderer-callable and
   * is asserted so thirty lines above. That matcher passed only while no renderer file called
   * the pre-flight at all, and would have failed the first correct implementation of §24.8's
   * *"rendered and confirmed in the renderer"*. A bar broader than its subject proves the wrong
   * thing; the subject here is the privileged command, so the pattern names it exactly.
   */
  it('routes the removal through the shell, never through a renderer-held command name', () => {
    const sources = appCode();
    expect(sources.length, 'the app scan read nothing').toBeGreaterThan(100);

    const privileged = /['"]locations\.uninstall['"]/u;
    const callers = sources
      .filter(({ path }) => path.startsWith('renderer/'))
      .filter(({ code }) => privileged.test(code))
      .map(({ path }) => path);

    expect(
      callers,
      'the renderer names a channel, never the privileged command the shell calls',
    ).toEqual([]);

    // The pre-flight is the other half of the same rule and must stay reachable: a matcher that
    // banned it would make this criterion refuse the verdict §24.8 requires the renderer to show.
    expect(privileged.test(`request('locations.uninstallPreflight', {})`)).toBe(false);
  });
});

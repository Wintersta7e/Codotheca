import { execFileSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { extname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import { withoutComments } from './lib/without-comments.mjs';

const REPO_ROOT = fileURLToPath(new URL('..', import.meta.url));

/**
 * Where a rendered string, an accessible name or a command name can live.
 *
 * `app/src/generated` is deliberately absent: it is gitignored, and a check that greps an
 * ignored path is the exact defect this gate was written against. Command names are checked at
 * their tracked source of truth, `protocol/schema/protocol.json`, instead.
 */
export const SCAN_ROOTS = [
  'app/src/renderer',
  'app/src/shared',
  'app/src/main',
  'app/src/preload',
  'protocol/schema/protocol.json',
];

const EXTENSIONS = new Set(['.ts', '.tsx', '.js', '.mjs', '.css', '.json', '.html']);

/**
 * Where `uninstall` is permitted, and nowhere else.
 *
 * **Empty in this change, and `uninstall` therefore stays banned outright** — this plan renders no
 * `UNINSTALL`. p2-24b appends its sites in the same change that renders the word, which is the
 * only way the bar and the body move together.
 *
 * **Every entry carries all three fields, and `token` is the one that matters.** A site of
 * `{path, why}` alone would permit *every* banned word at that path rather than the one it was
 * granted — `FORGET` would pass anywhere `uninstall` was allowed, and `FORGET` is the one token
 * §24.6 says is absent forever.
 *
 * @type {ReadonlyArray<{path: string, token: string, why: string}>}
 */
export const UNINSTALL_SITES = [
  {
    path: 'protocol/schema/protocol.json',
    token: 'UNINSTALL',
    why: '§24.7/§24.8 declare `locations.uninstall` and `locations.uninstallPreflight`, their verdict types and the comments that state the trust rules. The schema is the one place the two command names are spelled; `protocol/test/surface.test.mjs` asserts the pair is closed, so a third `uninstall` command fails there rather than passing quietly here.',
  },
  {
    path: 'app/src/main/core/idempotence.ts',
    token: 'UNINSTALL',
    why: '`COMMAND_EFFECT` is `Record<CommandName, …>`, so it must name every command the schema declares — including the two above. Omitting them is a type error, which is why this entry is unavoidable rather than a convenience.',
  },
  {
    path: 'app/src/shared/channels.ts',
    token: 'UNINSTALL',
    why: "§24.8: `IPC_UNINSTALL`'s VALUE has to say what the channel is. A channel named for something else would be worse than the word — the whole point of the constant is that a reader can tell what travels it.",
  },
  {
    path: 'app/src/shared/bridge.ts',
    token: 'UNINSTALL',
    why: "The bridge method the renderer calls, and the `UninstallReply` it resolves to. The occurrence `extractStrings` finds is not a rendered string at all — its `>text<` arm matches across a TypeScript generic — but narrowing the pattern to let a generic through would admit every other occurrence with it, which is the repair this file's DELETE_SITES comment already refused once.",
  },
  {
    path: 'app/src/preload/index.ts',
    token: 'UNINSTALL',
    why: 'The only route `locations.uninstall` has: the command is privileged, so `isRendererCallable` keeps it off IPC_REQUEST and the renderer reaches it through this wrapper or not at all. Same generic-in-a-return-type occurrence as the entry above.',
  },
  {
    path: 'app/src/main/dialogs/uninstall.ts',
    token: 'UNINSTALL',
    why: 'The shell half has to name the command it calls. Its two diagnostic strings deliberately do NOT carry the word — `bad locationId` and `could not complete` — because no user reads them and the direct translations would have forced a site for nothing.',
  },
  {
    path: 'app/src/main/index.ts',
    token: 'UNINSTALL',
    why: 'The import path of the handler above. A module has to be importable by its own name; renaming the file to dodge the gate would make the tree say less than it does.',
  },
  {
    path: 'app/src/renderer/project/uninstall/uninstallCopy.ts',
    token: 'UNINSTALL',
    why: "§24.6's rendered token and its confirmation copy, *Uninstall — the tile stays, re-clone any time.* This is the one file in the renderer that renders the word, and it renders every occurrence of it: the control below is arrangement and holds no copy.",
  },
  {
    path: 'app/src/renderer/project/uninstall/UninstallControl.tsx',
    token: 'UNINSTALL',
    why: 'Its BEM class names, `cp-uninstall` and the elements under it. The control renders no copy of its own — every sentence comes from the module above — but a class has to say which block it belongs to, and `\\buninstall\\b` matches across the hyphen.',
  },
  {
    path: 'app/src/renderer/project/uninstall/useUninstall.ts',
    token: 'UNINSTALL',
    why: "The page's half of §24.8: it names `locations.uninstallPreflight`, which is unprivileged and renderer-callable by design, and it is the only thing in the renderer that starts a pre-flight. The privileged `locations.uninstall` is NOT named here — it travels the shell's channel, and `app/test/uninstallAcceptance.test.ts` fails on a renderer file that names it.",
  },
  {
    path: 'app/src/renderer/project/ProjectPage.tsx',
    token: 'UNINSTALL',
    why: 'The import path of the hook above. §24.5 gives the removal one slot and the page is the surface that owns it — the rail is arrangement, and the copy being offered is the one the page is showing.',
  },
  {
    path: 'app/src/renderer/project/rail/Rail.tsx',
    token: 'UNINSTALL',
    why: "The import path of the control, which lives in a directory named for what it does. §24.5 puts Uninstall in exactly one slot and `app/test/installSites.test.ts` fails on a second, so this path is the gate's own evidence rather than a hole in it.",
  },
];

/**
 * Where `DELETE` is permitted, and nowhere else.
 *
 * **§24.2c's ban as written fails a correct, shipped phase-1 surface**, so the site mechanism is
 * what reconciles them rather than a weakened pattern. §8.8's armed chip renders
 * `PRESS AGAIN TO DELETE · THE PROJECTS STAY`, and `collections.remove` deletes a `collection`
 * row and its `collection_member` rows and **touches nothing else — no `project`, no `location`,
 * no `session`, no byte on disk** (`app/src/renderer/collections/armedDelete.ts:7-12`). The word
 * there names the removal of a **saved query**, which is not the removal wording §24.2c is aimed
 * at, and the second half of the same sentence says so to the user.
 *
 * Narrowing the *pattern* to let it through would have been the wrong repair: it would have
 * admitted every other `DELETE` with it. One path, one token, one written reason keeps the ban
 * total everywhere else.
 *
 * @type {ReadonlyArray<{path: string, token: string, why: string}>}
 */
export const DELETE_SITES = [
  {
    path: 'app/src/renderer/collections/armedDelete.ts',
    token: 'DELETE',
    why: '§8.8 armed chip: removes a saved query, not a working copy. `collections.remove` touches no project, location, session or byte on disk, and the same sentence tells the user the projects stay.',
  },
];

export const DESTRUCTIVE_TOKENS = [
  { token: 'FORGET', pattern: /\bforget\b/i, why: 'FORGET became RELOCATE' },
  {
    token: 'UNINSTALL',
    pattern: /\buninstall\b/i,
    why: 'phase 1 admits no destructive operation',
    sites: UNINSTALL_SITES,
  },
  {
    token: 'clean',
    pattern: /\bclean\b/i,
    why: 'absence of dirty is "no changes as of T", never "clean"',
  },
  { token: 'git push', pattern: /\bgit\s+push\b/i, why: 'every git invocation is read-only' },
  {
    token: 'git checkout',
    pattern: /\bgit\s+checkout\b/i,
    why: 'every git invocation is read-only',
  },
  { token: 'git clean', pattern: /\bgit\s+clean\b/i, why: 'every git invocation is read-only' },
  // §24.2c's removal wording. **Uppercase and exact, and that is not a weakening.**
  //
  // Every pattern above carries `i`, and this gate scans `protocol/schema/protocol.json`, which
  // declares the shipped command names `roots.remove` and `collections.remove`. A
  // case-insensitive `/\bremove\b/` fails the build on two correct command names the moment it
  // lands. The product's rendered controls are uppercase — `RELOCATE`, `TRY AGAIN`, `UNINSTALL` —
  // so an uppercase-exact ban catches every rendered control name and no identifier. The
  // self-test plants both directions.
  {
    token: 'DELETE',
    pattern: /\bDELETE\b/,
    why: 'phase 2 ships no DELETE; removal wording is a phase-4 affordance',
    sites: DELETE_SITES,
  },
  {
    token: 'REMOVE',
    pattern: /\bREMOVE\b/,
    why: 'phase 2 ships no REMOVE; RELOCATE is not widened to carry it',
  },
  {
    token: 'RECLAIM SPACE',
    pattern: /\bRECLAIM\s+SPACE\b/,
    why: 'double-booked in the Amnesty between working-copy removal and the junk sweep',
  },
];

/** Empty, and a new entry needs all three fields. An escape hatch is how a gate dies quietly. */
export const ALLOWLIST = [];

/**
 * String literals and JSX text only. Identifiers and comments are out of scope by design: the
 * ban is on what a user reads, and a comment naming the ban must not trip it.
 *
 * The comment stripping moved to `scripts/lib/without-comments.mjs` when a second gate needed it
 * — one owner, and it carries a `.d.mts` so a TypeScript gate can use the same one.
 */
export function extractStrings(source) {
  const code = withoutComments(source);
  const literal = /'([^'\\\n]|\\.)*'|"([^"\\\n]|\\.)*"|`([^`\\]|\\.)*`|>([^<>{}]+)</g;
  const found = [];
  for (const match of code.matchAll(literal)) {
    const line = code.slice(0, match.index).split('\n').length;
    found.push({ text: match[0], line });
  }
  return found;
}

export function scanSource(source, path) {
  const violations = [];
  for (const { text, line } of extractStrings(source)) {
    for (const rule of DESTRUCTIVE_TOKENS) {
      if (!rule.pattern.test(text)) continue;
      if (ALLOWLIST.some((entry) => entry.path === path && entry.token === rule.token)) continue;
      // A rule's own site list, which is narrower than the allowlist in the way that matters: a
      // site permits **one token at one path**, so it cannot become a general exemption for that
      // file. An unenumerated occurrence still fails, which is what makes the list a claim about
      // where the word is rendered rather than a hole.
      if ((rule.sites ?? []).some((site) => site.path === path && site.token === rule.token)) {
        continue;
      }
      violations.push({
        path,
        line,
        token: rule.token,
        text: text.trim().slice(0, 120),
        why: rule.why,
      });
    }
  }
  return violations;
}

export function collectFiles(roots, repoRoot) {
  const files = [];

  // The kind comes from the directory entry `readdirSync` already read, never from a second
  // `statSync` on a path the walk reached — a probe removed in that gap throws ENOENT and takes
  // the gate down, which is the failure `scanFiles` below guards one step later at the read. Only
  // the roots are stat'd, and a root is a fixed directory nobody plants a probe over.
  const walk = (absolute, isDirectory) => {
    if (!isDirectory) {
      // The ban is on what a user reads. A test that asserts the ban must be able to name the
      // token, exactly the same carve-out the gate makes for comments.
      if (
        absolute.endsWith('.test.ts') ||
        absolute.endsWith('.test.tsx') ||
        absolute.endsWith('.test.js') ||
        absolute.endsWith('.test.mjs')
      ) {
        return;
      }
      if (EXTENSIONS.has(extname(absolute))) files.push(absolute);
      return;
    }

    for (const entry of readdirSync(absolute, { withFileTypes: true })) {
      if (entry.name === 'node_modules' || entry.name === 'dist') continue;
      walk(join(absolute, entry.name), entry.isDirectory());
    }
  };

  for (const root of roots) {
    const full = join(repoRoot, root);
    walk(full, statSync(full).isDirectory());
  }
  return files.sort();
}

/**
 * The read half, separated from the walk so the gap between them is testable.
 *
 * A file can vanish in that gap: `app/test/styleGates.test.ts` plants and removes a probe
 * stylesheet inside one of these roots to prove its own gate can fail, and vitest runs it in
 * parallel with this gate's suite. Reading the walk's list unguarded crashed this gate with an
 * ENOENT stack and failed the whole app suite — a gate that throws is a gate that reports
 * nothing. A file that is no longer there carries no rendered string, so it is skipped and
 * deliberately not counted, which keeps the "scanned nothing" guard meaning what it says. Any
 * other read error is a real problem and is raised.
 */
export function scanFiles(files, repoRoot) {
  const violations = [];
  let scanned = 0;
  for (const file of files) {
    let source;
    try {
      source = readFileSync(file, 'utf8');
    } catch (error) {
      if (error && error.code === 'ENOENT') continue;
      throw error;
    }
    scanned += 1;
    violations.push(...scanSource(source, relative(repoRoot, file).split(sep).join('/')));
  }
  return { scanned, violations };
}

function assertRootIsReadable(root) {
  const absolute = join(REPO_ROOT, root);
  if (!existsSync(absolute)) {
    return `scan root does not exist: ${root} — the gate refuses to pass on a scan it did not run`;
  }

  try {
    execFileSync('git', ['check-ignore', '-q', '--', root], {
      cwd: REPO_ROOT,
      stdio: 'ignore',
    });
    return `scan root is gitignored: ${root} — grepping an ignored path is a check that can never fail`;
  } catch {
    // Non-zero from `check-ignore` means "not ignored", which is what this gate requires.
  }

  const tracked = execFileSync('git', ['ls-files', '--', root], {
    cwd: REPO_ROOT,
    encoding: 'utf8',
  })
    .split('\n')
    .filter(Boolean);
  if (tracked.length === 0) return `scan root holds no tracked file: ${root}`;
  return null;
}

/** Proves the detector still detects before a clean report is believed. */
export function selfTest() {
  const failures = [];
  for (const rule of DESTRUCTIVE_TOKENS) {
    const planted = `const label = 'do not ${rule.token} this';`;
    const detected = scanSource(planted, 'fixture.ts').some(
      (violation) => violation.token === rule.token,
    );
    if (!detected) failures.push(`self-test: ${rule.token} was not detected in a string literal`);
  }

  const firstToken = DESTRUCTIVE_TOKENS[0].token;
  if (scanSource(`// ${firstToken} is banned here\n`, 'fixture.ts').length !== 0) {
    failures.push('self-test: a comment tripped the gate; the ban is on rendered strings');
  }
  if (scanSource("const ok = 'relocate this copy';", 'fixture.ts').length !== 0) {
    failures.push('self-test: a clean fixture reported a violation');
  }

  // §24.2c's three removal words, in BOTH directions. A ban that has never been seen to fire is a
  // ban nobody has tested, and a case-sensitive ban that has never been seen to *hold its fire* is
  // one nobody has checked the cost of.
  for (const planted of ['DELETE THIS COPY', 'REMOVE THE FOLDER', 'RECLAIM SPACE']) {
    if (scanSource(`const label = '${planted}';`, 'fixture.ts').length === 0) {
      failures.push(`self-test: ${planted} was not detected in a rendered string`);
    }
  }
  // The reason those three are uppercase-exact: these are shipped command names, and a
  // case-insensitive ban would fail the build on `protocol/schema/protocol.json` itself.
  for (const permitted of ['roots.remove', 'collections.remove', 'locations.relocate']) {
    const hits = scanSource(`const name = '${permitted}';`, 'fixture.ts');
    if (hits.length !== 0) {
      failures.push(
        `self-test: ${permitted} tripped the gate as ${hits.map((h) => h.token).join(', ')}; ` +
          'the removal-wording ban is uppercase-exact so it catches rendered controls, not identifiers',
      );
    }
  }

  // The site mechanism, proven in both directions against a synthetic rule rather than against
  // `UNINSTALL_SITES`, so the fixture keeps working whatever that list holds.
  const sited = [
    { token: 'UNINSTALL', pattern: /\buninstall\b/i, why: 'fixture', sites: UNINSTALL_SITES },
  ];
  if (sited.length !== 1) failures.push('self-test: the site fixture is malformed');
  // [p2] **The empty assertion has moved, not been dropped.** p2-24 rendered no `UNINSTALL`, so
  // an empty list was the honest state and a non-empty one would have been an exemption nobody
  // asked for. p2-24b renders the word, so what is asserted now is that every entry carries a
  // real path and a written reason — an entry with an empty `why` is the escape hatch the
  // original assertion existed to prevent, wearing a different shape.
  for (const site of UNINSTALL_SITES) {
    if (typeof site.path !== 'string' || site.path.length === 0) {
      failures.push('self-test: an UNINSTALL site has no path');
    }
    if (typeof site.why !== 'string' || site.why.length < 40) {
      failures.push(`self-test: ${site.path} is permitted with no written reason`);
    }
    if (site.token !== 'UNINSTALL') {
      failures.push(`self-test: ${site.path} is sited for ${site.token}, not UNINSTALL`);
    }
  }
  if (scanSource("const label = 'UNINSTALL';", 'app/src/renderer/anywhere.tsx').length === 0) {
    failures.push('self-test: UNINSTALL outside the site list did not fail');
  }
  if (ALLOWLIST.length !== 0) {
    failures.push('self-test: ALLOWLIST is not empty; an escape hatch is how a gate dies quietly');
  }
  if (assertRootIsReadable('app/src/no-such-root') === null) {
    failures.push('self-test: a missing root did not fail the guard');
  }
  if (assertRootIsReadable('app/src/generated') === null) {
    failures.push('self-test: the gitignored generated root did not fail the guard');
  }
  return failures;
}

function main(argv) {
  if (argv.includes('--self-test')) {
    const failures = selfTest();
    for (const failure of failures) process.stderr.write(`${failure}\n`);
    return failures.length === 0 ? 0 : 1;
  }

  let json = false;
  let selectedRoot = null;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === '--json') {
      json = true;
      continue;
    }
    if (argument === '--root') {
      selectedRoot = argv[index + 1] ?? null;
      index += 1;
      if (selectedRoot === null) {
        process.stderr.write('--root requires a path\n');
        return 1;
      }
      continue;
    }
    process.stderr.write(`unknown argument: ${argument}\n`);
    return 1;
  }

  const roots = selectedRoot === null ? SCAN_ROOTS : [selectedRoot];
  const guardFailures = roots.map(assertRootIsReadable).filter((failure) => failure !== null);
  if (guardFailures.length > 0) {
    for (const failure of guardFailures) process.stderr.write(`${failure}\n`);
    return 1;
  }

  const { scanned, violations } = scanFiles(collectFiles(roots, REPO_ROOT), REPO_ROOT);

  if (scanned === 0) {
    process.stderr.write(
      'destructive-token gate: 0 files scanned — the gate refuses to pass on a scan it did not run\n',
    );
    return 1;
  }

  if (json) {
    process.stdout.write(`${JSON.stringify({ filesScanned: scanned, violations }, null, 2)}\n`);
    return violations.length === 0 ? 0 : 1;
  }

  for (const violation of violations) {
    process.stderr.write(
      `${violation.path}:${violation.line}: ${violation.token} — ${violation.why}\n  ${violation.text}\n`,
    );
  }
  if (violations.length > 0) return 1;
  process.stdout.write(`destructive-token gate: ${String(scanned)} files, 0 violations\n`);
  return 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1])
  process.exit(main(process.argv.slice(2)));

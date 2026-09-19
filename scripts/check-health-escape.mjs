// [p3] §35.8.6 / `AC-P3-35-7`: **no health-derived figure leaves the window.**
//
// No count reaches a notification, a window title, a taskbar, a tray or an app icon. The tray
// icon is *never badged, never animated, never red*, and a surface may not wear what the tray is
// forbidden to wear. The notification contract permits exactly three and none of them is this;
// **no notification ever mentions absence**, which this surface is largely made of.
//
// **A violation is co-location within one file.** A per-expression dataflow claim is not
// something a scanner can honestly make, and pretending otherwise is the *bar written past its
// defect* shape. Both hit lists are reported with line numbers so the reviewer sees what was
// actually found.
//
// **A gate whose passing run scans zero files is a failing gate**, so the count is printed and a
// zero exits non-zero — and the same holds for the derived identifier set, which would otherwise
// pass over everything the first time §30 renamed a type.
import { readdirSync, statSync } from 'node:fs';
import { extname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import { readScannedFile } from './lib/read-scanned.mjs';
import { withoutComments } from './lib/without-comments.mjs';

const REPO_ROOT = fileURLToPath(new URL('..', import.meta.url));

/** The shell and the renderer both. An escape API lives in `main`; the figure comes from either. */
export const SCAN_ROOTS = ['app/src/main', 'app/src/preload', 'app/src/renderer', 'app/src/shared'];

const EXTENSIONS = new Set(['.ts', '.tsx', '.js', '.mjs']);

/** The two §30 types whose values are the figures this gate is about. */
const HEALTH_TYPES = ['HealthSummary', 'HealthReading'];

/**
 * Where a health-derived count can leave the window, **matched by call shape rather than by bare
 * word**.
 *
 * The criterion names its own trap: *the substring it bans must not also match the thing it is
 * meant to allow*. `app/src/main/paletteShortcut.ts` exports `activateFromTray`, which contains
 * the substring `Tray` and is entirely legitimate; every entry below carries the punctuation that
 * makes it a call or a property write, so a name cannot hit.
 */
export const ESCAPE_CALLS = [
  'new Notification(',
  'new Tray(',
  '.setOverlayIcon(',
  '.setBadgeCount(',
  '.setProgressBar(',
  '.flashFrame(',
  '.setToolTip(',
  '.setTitle(',
  'app.dock.setBadge(',
  'document.title',
];

/**
 * The banned identifier set, **derived from the schema and never listed**.
 *
 * A gate with a hand-kept list goes stale the first time §30 adds a field. Three derivations,
 * each from `protocol/schema/protocol.json`:
 *
 * 1. the two type names;
 * 2. every **carrier** — a struct field declared as one of those types, which is how a consumer
 *    reaches the value at all;
 * 3. every field of those two types that appears on **no other type**.
 *
 * Rule 3's set difference is what keeps the gate honest. `state`, `basis` and `observedAt` are
 * field names of other wire types too, so matching them bare names nothing about health — and it
 * would report the one legitimate notification site in `app/src/main/index.ts`, which is the
 * criterion's own trap wearing an identifier instead of a call. A carrier or a type name has to
 * appear in the file for the value to be reachable there, including through a destructuring, so
 * dropping the three shared names loses no reachable case.
 */
export function healthIdentifiers(schema) {
  const types = schema.types ?? {};
  const own = new Set();
  for (const name of HEALTH_TYPES) {
    for (const field of Object.keys(types[name]?.fields ?? {})) own.add(field);
  }
  const elsewhere = new Set();
  const carriers = new Set();
  for (const [name, declared] of Object.entries(types)) {
    const fields = declared.fields;
    if (fields === undefined) continue;
    for (const [field, type] of Object.entries(fields)) {
      if (!HEALTH_TYPES.includes(name)) elsewhere.add(field);
      if (HEALTH_TYPES.includes(String(type).replace(/\?$/u, ''))) carriers.add(field);
    }
  }
  const unique = [...own].filter((field) => !elsewhere.has(field));
  return [...new Set([...HEALTH_TYPES, ...carriers, ...unique])].sort();
}

export function collectFiles(roots, repoRoot) {
  const files = [];
  // The kind comes from the directory entry `readdirSync` already read, never from a second
  // `statSync` on a path the walk reached: a probe removed in that gap throws ENOENT and takes
  // the gate down. Only the roots are stat'd, and a root is a fixed directory.
  const walk = (absolute, isDirectory) => {
    if (!isDirectory) {
      if (EXTENSIONS.has(extname(absolute))) files.push(absolute);
      return;
    }
    for (const entry of readdirSync(absolute, { withFileTypes: true })) {
      if (entry.name === 'node_modules' || entry.name === 'dist') continue;
      walk(join(absolute, entry.name), entry.isDirectory());
    }
  };
  for (const root of roots)
    walk(join(repoRoot, root), statSync(join(repoRoot, root)).isDirectory());
  return files.sort();
}

function hitsIn(code, needles) {
  const hits = [];
  const lines = code.split('\n');
  for (const [index, text] of lines.entries()) {
    for (const needle of needles) {
      if (text.includes(needle)) hits.push({ line: index + 1, token: needle });
    }
  }
  return hits;
}

/** Word-boundary, over comment-stripped source: a comment naming an identifier is not a figure. */
function identifierHits(code, identifiers) {
  const hits = [];
  for (const [index, text] of code.split('\n').entries()) {
    for (const identifier of identifiers) {
      if (new RegExp(`\\b${identifier}\\b`, 'u').test(text)) {
        hits.push({ line: index + 1, token: identifier });
      }
    }
  }
  return hits;
}

export function scanSource(source, path, identifiers) {
  const code = withoutComments(source);
  const escapes = hitsIn(code, ESCAPE_CALLS);
  const figures = identifierHits(code, identifiers);
  // Co-location within one file, stated as exactly that. A file holding an escape call and no
  // health identifier is **not** a violation: a gate that flagged every notification would be
  // deleted by the first person who needed one.
  const violations =
    escapes.length > 0 && figures.length > 0 ? [{ path, escapes, identifiers: figures }] : [];
  return { escapes: escapes.map((hit) => ({ path, ...hit })), violations };
}

export function scanFiles(files, repoRoot, identifiers) {
  const escapeSites = [];
  const violations = [];
  let scanned = 0;
  for (const file of files) {
    const source = readScannedFile(file);
    // A file that vanished between the walk and the read carries nothing to check and is **not
    // counted** — counting it would make the "scanned nothing" guard stop meaning what it says.
    if (source === null) continue;
    scanned += 1;
    const path = relative(repoRoot, file).split(sep).join('/');
    const result = scanSource(source, path, identifiers);
    escapeSites.push(...result.escapes);
    violations.push(...result.violations);
  }
  return { scanned, escapeSites, violations };
}

export function main(argv, repoRoot = REPO_ROOT) {
  const json = argv.includes('--json');
  // `--root <dir>` scans one absolute directory instead of `SCAN_ROOTS`. **The probes that prove
  // this gate bites go there, not into `app/src/renderer`**: vitest runs the node project's files
  // in parallel, a probe planted in a tree other gates are walking is read by one of them
  // mid-life, and `new Notification(` inside the renderer is a live violation of
  // `check-notification-origin.mjs` while it sits there
  // (`app/test/notificationOrigin.test.ts:24-26` records the same decision).
  const rootIndex = argv.indexOf('--root');
  const selectedRoot = rootIndex === -1 ? null : (argv[rootIndex + 1] ?? null);
  if (rootIndex !== -1 && selectedRoot === null) {
    process.stderr.write('--root requires a path\n');
    return 1;
  }
  const schemaText = readScannedFile(join(REPO_ROOT, 'protocol/schema/protocol.json'));
  if (schemaText === null) {
    process.stderr.write('health-escape gate: protocol/schema/protocol.json is not readable\n');
    return 1;
  }
  const identifiers = healthIdentifiers(JSON.parse(schemaText));
  if (identifiers.length === 0) {
    process.stderr.write(
      'health-escape gate: 0 identifiers derived — a renamed type would make this gate pass ' +
        'over everything\n',
    );
    return 1;
  }

  const base = selectedRoot === null ? repoRoot : selectedRoot;
  const roots = selectedRoot === null ? SCAN_ROOTS : ['.'];
  const { scanned, escapeSites, violations } = scanFiles(
    collectFiles(roots, base),
    base,
    identifiers,
  );
  if (scanned === 0) {
    process.stderr.write(
      'health-escape gate: 0 files scanned — the gate refuses to pass on a scan it did not run\n',
    );
    return 1;
  }

  if (json) {
    process.stdout.write(
      `${JSON.stringify({ filesScanned: scanned, identifiers, escapeSites, violations }, null, 2)}\n`,
    );
    return violations.length === 0 ? 0 : 1;
  }

  for (const violation of violations) {
    process.stderr.write(
      `${violation.path}: a health-derived figure sits beside an escape call\n` +
        violation.escapes.map((h) => `  :${h.line} ${h.token}\n`).join('') +
        violation.identifiers.map((h) => `  :${h.line} ${h.token}\n`).join(''),
    );
  }
  if (violations.length > 0) return 1;
  process.stdout.write(
    `health-escape gate: ${String(scanned)} files, ${String(identifiers.length)} identifiers ` +
      `(${identifiers.join(', ')}), ${String(escapeSites.length)} escape site(s), 0 violations\n`,
  );
  return 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1])
  process.exit(main(process.argv.slice(2)));

/**
 * [p4] §43's static gates over the renderer and the shell: one read, the level gates nothing, no
 * sound.
 *
 * Every scan reads through `scripts/lib/read-scanned.mjs`, so a file that vanished between the
 * walk and the read is skipped **before it is counted** — other gates plant probes in these trees
 * while vitest runs this project in parallel. Each prints what it scanned and fails at zero: a
 * gate whose passing run read nothing is a failing gate. Comments are stripped first, so a
 * sentence naming a rule never trips it. Test files are not product and are not scanned.
 */
import { readFileSync } from 'node:fs';
import { relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from 'vitest';

import { collectFiles } from '../../scripts/check-health-escape.mjs';
import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';
import { withoutComments } from '../../scripts/lib/without-comments.mjs';

const REPO = fileURLToPath(new URL('../..', import.meta.url));

interface Source {
  readonly path: string;
  readonly code: string;
}

/** Product sources under `roots`, repo-relative and comment-stripped; vanished files dropped. */
function sources(roots: readonly string[]): Source[] {
  return collectFiles(roots, REPO).flatMap((file) => {
    const path = relative(REPO, file).split(sep).join('/');
    if (/\.test\.tsx?$/u.test(path) || path.startsWith('app/src/generated/')) return [];
    const text = readScannedFile(file);
    return text === null ? [] : [{ path, code: withoutComments(text) }];
  });
}

/** Lines of `code` matching `pattern`, as `path:line`. */
function hitLines(source: Source, pattern: RegExp): string[] {
  return source.code
    .split('\n')
    .flatMap((text, index) => (pattern.test(text) ? [`${source.path}:${String(index + 1)}`] : []));
}

const within = (path: string, allowed: readonly string[]): boolean =>
  allowed.some((entry) => (entry.endsWith('/') ? path.startsWith(entry) : path === entry));

/** §42's tray. `residency.ts` holds it today; §42's own tray file joins this list when it lands. */
const TRAY_MODULES = ['app/src/main/residency.ts'];

/** §43.15's one read of the layer. Everything else is handed the state or hears the event. */
const READ_HOOK = 'app/src/renderer/motivation/useMotivation.ts';

const MOTIVATION_GET = /request\(\s*['"]motivation\.get['"]/u;

// [p4] §43.15: the layer is read once and then heard; a timer reading it again is a poll.
test('AC-P4-43-1 no polling read', () => {
  const timed = sources(['app/src/renderer/motivation', ...TRAY_MODULES]);
  const requesting = sources(['app/src/renderer', 'app/src/main']);
  const hits = [
    ...timed.flatMap((source) => hitLines(source, /\bsetInterval\b/u)),
    ...requesting
      .filter((source) => source.path !== READ_HOOK)
      .flatMap((source) => hitLines(source, MOTIVATION_GET)),
  ];
  console.warn(
    `AC-P4-43-1: ${String(timed.length)} band/block/tray file(s) scanned for setInterval, ` +
      `${String(requesting.length)} renderer/shell file(s) for motivation.get; ` +
      `${String(hits.length)} hit(s)`,
  );
  expect(timed.length, 'a scan that read nothing proves nothing').toBeGreaterThan(0);
  expect(requesting.length).toBeGreaterThan(timed.length);
  // The one read is where this gate says it is, so the exemption cannot outlive it.
  const hook = requesting.find((source) => source.path === READ_HOOK);
  expect(hook === undefined ? [] : hitLines(hook, MOTIVATION_GET)).toHaveLength(1);
  expect(hits).toEqual([]);
});

/** The names a reader of the level or the layer's state has to spell. */
const READER = /\b(?:LevelReading|MotivationState|pendingLine|xpBar)\b/u;

/**
 * `terms` is also `QueryAst`'s field, which the query modules read as `ast.terms`; a bare word
 * would name the query, not the level. A `.terms` read through any other receiver is the layer's.
 */
const TERMS_READ = /\b(?!ast\.)[\w$]+\.terms\b/u;

/**
 * Where the level and the layer's state may be read: the presentation modules, and the
 * composition seams that hand the state on — `FirstRunHost` and `FirstRunGate` carry it to the
 * reveal.
 */
const READERS_ALLOWED = [
  'app/src/renderer/motivation/',
  'app/src/renderer/profile/',
  'app/src/renderer/firstrun/revealMotivation.ts',
  'app/src/renderer/firstrun/RevealScreen.tsx',
  'app/src/renderer/firstrun/FirstRunGate.tsx',
  'app/src/renderer/App.tsx',
  'app/src/renderer/app/route.ts',
  'app/src/renderer/app/ShelfScreen.tsx',
  'app/src/renderer/app/FirstRunHost.tsx',
  ...TRAY_MODULES,
];

/** Only the modules that draw the number may compare it with anything; a seam only hands it on. */
const COMPARISONS_ALLOWED = ['app/src/renderer/motivation/', 'app/src/renderer/profile/'];

/** A `level` that is not §38's, each with what it is. The gate asserts each still matches. */
const OTHER_LEVELS: Readonly<Record<string, string>> = {
  'app/src/renderer/shelf/useShedLevel.ts': "the top bar's shed level, §8.0a",
};

const OPS = '(?:===|!==|==|!=|>=|<=|>|<)';
const LEVEL_VS_LITERAL = new RegExp(
  `[\\w$.]*[lL]evel\\b\\s*${OPS}\\s*-?\\d|\\d\\s*${OPS}\\s*[\\w$.]*[lL]evel\\b`,
  'u',
);

// [p4] §43.12: the level is a presentation; no surface branches on it.
test('AC-P4-43-19 the level gates nothing', () => {
  const scanned = sources(['app/src']);
  const readers = scanned.filter(
    (source) => READER.test(source.code) || TERMS_READ.test(source.code),
  );
  const strayReaders = readers.map((s) => s.path).filter((p) => !within(p, READERS_ALLOWED));
  const compared = scanned.flatMap((source) =>
    within(source.path, COMPARISONS_ALLOWED) ? [] : hitLines(source, LEVEL_VS_LITERAL),
  );
  const otherLevels = Object.keys(OTHER_LEVELS);
  const gating = compared.filter((hit) => !otherLevels.some((path) => hit.startsWith(`${path}:`)));
  console.warn(
    `AC-P4-43-19: ${String(scanned.length)} file(s) scanned, ${String(readers.length)} reader(s) ` +
      `(${readers.map((s) => s.path).join(', ')}); ${String(gating.length)} comparison(s)`,
  );
  expect(scanned.length, 'a scan that read nothing proves nothing').toBeGreaterThan(0);
  expect(readers.length, 'no reader found means the names drifted').toBeGreaterThan(0);
  expect(strayReaders).toEqual([]);
  for (const path of otherLevels) {
    expect(
      compared.some((hit) => hit.startsWith(`${path}:`)),
      `${path} is stale`,
    ).toBe(true);
  }
  expect(gating).toEqual([]);
});

const SOUND = /AudioContext|new Audio\(|<audio\b|shell\.beep\(/u;

// [p4] §43.16: the layer makes no sound, and nothing in the app can.
test('AC-P4-43-21 no sound', () => {
  const scanned = sources(['app/src']);
  const hits = scanned.flatMap((source) => hitLines(source, SOUND));
  const schema: unknown = JSON.parse(
    readFileSync(
      fileURLToPath(new URL('../../protocol/schema/protocol.json', import.meta.url)),
      'utf8',
    ),
  );
  const fields = Object.keys(
    (schema as { types: { Settings: { fields: Record<string, string> } } }).types.Settings.fields,
  );
  const soundFields = fields.filter((field) => /sound|audio/iu.test(field));
  console.warn(
    `AC-P4-43-21: ${String(scanned.length)} file(s) scanned, ${String(hits.length)} hit(s); ` +
      `Settings has ${String(fields.length)} field(s), ${String(soundFields.length)} naming sound`,
  );
  expect(scanned.length, 'a scan that read nothing proves nothing').toBeGreaterThan(0);
  expect(fields.length).toBeGreaterThan(0);
  expect(hits).toEqual([]);
  expect(soundFields).toEqual([]);
});

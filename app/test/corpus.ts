/**
 * Reads the corpus the Rust generator writes. Nothing here builds fixtures; it locates them
 * and validates that the manifest still has the shape this file was written against.
 *
 * The manifest is written by Rust and read here, so the two shapes can drift silently. That is
 * what `parseCorpusManifest` exists to stop: it names the offending key, and a test asserts
 * every id below appears in a real generated manifest, so a Rust-side rename fails here rather
 * than producing `undefined` three plans later.
 */
import { execFileSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';

/** Must equal `codotheca_core::corpus::CORPUS_VERSION`. */
export const CORPUS_VERSION = 1;

export interface CorpusVolume {
  id: string;
  path: string;
  storeKey: string;
  volumeKey: string;
  class: string;
}

export interface FixtureExpect {
  bare: boolean;
  shallow: boolean;
  headOid: string | null;
  rootOids: string[];
  historyDepth: number | null;
  originUrl: string | null;
  parentFixture: string | null;
  submodulePath: string | null;
  indexLockHeld: boolean;
  requiresLongPaths: boolean;
  untrackedFiles: number;
  notes: string | null;
}

export interface CorpusFixture {
  name: string;
  volume: string;
  path: string;
  materialised: boolean;
  skipReason: string | null;
  expect: FixtureExpect;
}

export interface CorpusManifest {
  corpusVersion: number;
  gitVersion: string;
  root: string;
  volumes: CorpusVolume[];
  fixtures: CorpusFixture[];
}

/** One entry per id in `codotheca_core::corpus::fixtures::ORDER`. */
export const FIXTURES = {
  upstream: 'upstream',
  otherUpstream: 'other-upstream',
  zeroCommit: 'zero-commit',
  bare: 'bare',
  shallow: 'shallow',
  multiRoot: 'multi-root',
  futureDated: 'future-dated',
  indexLockHeld: 'index-lock-held',
  hugeUntracked: 'huge-untracked',
  fork: 'fork',
  copyOne: 'copy-one',
  copyTwo: 'copy-two',
  ambiguousLineage: 'ambiguous-lineage',
  repoInsideRepoOuter: 'repo-inside-repo-outer',
  repoInsideRepoInner: 'repo-inside-repo-inner',
  worktreeParent: 'worktree-parent',
  linkedWorktree: 'linked-worktree',
  submoduleParent: 'submodule-parent',
  submoduleChild: 'submodule-child',
  submoduleNested: 'submodule-nested',
  nonUtf8Path: 'non-utf8-path',
  longPath: 'long-path',
  symlinkCycle: 'symlink-cycle',
  dubiousOwnership: 'dubious-ownership',
  deepHistory: 'deep-history',
} as const;

function fail(key: string, why: string): never {
  throw new Error(`corpus manifest: ${key} ${why}`);
}

function record(value: unknown, key: string): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    return fail(key, 'is not an object');
  }
  return value as Record<string, unknown>;
}

function str(source: Record<string, unknown>, key: string, at: string): string {
  const value = source[key];
  if (typeof value !== 'string') return fail(`${at}.${key}`, 'is not a string');
  return value;
}

function strOrNull(source: Record<string, unknown>, key: string, at: string): string | null {
  const value = source[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'string') return fail(`${at}.${key}`, 'is not a string or null');
  return value;
}

function num(source: Record<string, unknown>, key: string, at: string): number {
  const value = source[key];
  if (typeof value !== 'number') return fail(`${at}.${key}`, 'is not a number');
  return value;
}

function numOrNull(source: Record<string, unknown>, key: string, at: string): number | null {
  const value = source[key];
  if (value === null || value === undefined) return null;
  if (typeof value !== 'number') return fail(`${at}.${key}`, 'is not a number or null');
  return value;
}

function bool(source: Record<string, unknown>, key: string, at: string): boolean {
  const value = source[key];
  if (typeof value !== 'boolean') return fail(`${at}.${key}`, 'is not a boolean');
  return value;
}

function parseExpect(value: unknown, at: string): FixtureExpect {
  const raw = record(value, at);
  const rootOids = raw['rootOids'];
  if (!Array.isArray(rootOids) || rootOids.some((o) => typeof o !== 'string')) {
    return fail(`${at}.rootOids`, 'is not an array of strings');
  }
  return {
    bare: bool(raw, 'bare', at),
    shallow: bool(raw, 'shallow', at),
    headOid: strOrNull(raw, 'headOid', at),
    rootOids: rootOids as string[],
    historyDepth: numOrNull(raw, 'historyDepth', at),
    originUrl: strOrNull(raw, 'originUrl', at),
    parentFixture: strOrNull(raw, 'parentFixture', at),
    submodulePath: strOrNull(raw, 'submodulePath', at),
    indexLockHeld: bool(raw, 'indexLockHeld', at),
    requiresLongPaths: bool(raw, 'requiresLongPaths', at),
    untrackedFiles: num(raw, 'untrackedFiles', at),
    notes: strOrNull(raw, 'notes', at),
  };
}

export function parseCorpusManifest(value: unknown): CorpusManifest {
  const raw = record(value, 'manifest');
  const corpusVersion = num(raw, 'corpusVersion', 'manifest');
  if (corpusVersion !== CORPUS_VERSION) {
    return fail(
      'manifest.corpusVersion',
      `is ${String(corpusVersion)}, expected ${String(CORPUS_VERSION)}`,
    );
  }
  // Scalars first, in declaration order: this parser exists to name the key that is wrong, and
  // checking the arrays first reports `volumes` for a manifest whose real defect is a missing
  // `gitVersion`.
  const gitVersion = str(raw, 'gitVersion', 'manifest');
  const root = str(raw, 'root', 'manifest');
  const volumesRaw = raw['volumes'];
  if (!Array.isArray(volumesRaw)) return fail('manifest.volumes', 'is not an array');
  const fixturesRaw = raw['fixtures'];
  if (!Array.isArray(fixturesRaw)) return fail('manifest.fixtures', 'is not an array');

  return {
    corpusVersion,
    gitVersion,
    root,
    volumes: volumesRaw.map((entry, index) => {
      const at = `volumes[${String(index)}]`;
      const item = record(entry, at);
      return {
        id: str(item, 'id', at),
        path: str(item, 'path', at),
        storeKey: str(item, 'storeKey', at),
        volumeKey: str(item, 'volumeKey', at),
        class: str(item, 'class', at),
      };
    }),
    fixtures: fixturesRaw.map((entry, index) => {
      const at = `fixtures[${String(index)}]`;
      const item = record(entry, at);
      return {
        name: str(item, 'name', at),
        volume: str(item, 'volume', at),
        path: str(item, 'path', at),
        materialised: bool(item, 'materialised', at),
        skipReason: strOrNull(item, 'skipReason', at),
        expect: parseExpect(item['expect'], `${at}.expect`),
      };
    }),
  };
}

/**
 * Locate the corpus, building it with the Rust generator if it is not already there.
 * `CODOTHECA_CORPUS_DIR` lets CI build once and share the directory between jobs.
 */
export function ensureCorpus(outDir?: string): CorpusManifest {
  const fromEnv = process.env['CODOTHECA_CORPUS_DIR'];
  const dir =
    outDir ?? fromEnv ?? path.join(os.tmpdir(), `codotheca-corpus-v${String(CORPUS_VERSION)}`);
  const manifestPath = path.join(dir, 'manifest.json');
  if (!fs.existsSync(manifestPath)) {
    const appDir = fileURLToPath(new URL('..', import.meta.url));
    const repoRoot = path.resolve(appDir, '..');
    execFileSync(
      'cargo',
      [
        'run',
        '--quiet',
        '--manifest-path',
        path.join(repoRoot, 'core', 'Cargo.toml'),
        '--features',
        'testkit',
        '--bin',
        'codotheca-corpus',
        '--',
        '--out',
        dir,
      ],
      { stdio: 'inherit' },
    );
  }
  return parseCorpusManifest(JSON.parse(fs.readFileSync(manifestPath, 'utf8')) as unknown);
}

/** The accessor a test should use: an unavailable fixture throws with its reason. */
export function requireFixture(manifest: CorpusManifest, name: string): CorpusFixture {
  const fixture = manifest.fixtures.find((f) => f.name === name);
  if (fixture === undefined) throw new Error(`fixture unavailable: ${name} is not in the manifest`);
  if (!fixture.materialised) {
    throw new Error(`fixture unavailable: ${name}: ${fixture.skipReason ?? 'unknown'}`);
  }
  return fixture;
}

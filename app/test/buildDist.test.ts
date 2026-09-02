/**
 * The build script's contract, asserted without running a build.
 *
 * Three things here are worth a test rather than a reading. A cross-compile must be refused
 * rather than silently produced, because an artifact built for a platform it never ran on is
 * the thing this whole script exists to prevent. Every remedy string has to name a command the
 * reader can run — a remedy that only describes the problem is what made the native-binding
 * failures cost days. And the two shell wrappers must delegate rather than reimplement, or the
 * two platforms drift apart without anything saying so.
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const REPO = fileURLToPath(new URL('../..', import.meta.url));

interface BuildDist {
  readonly targetsFor: (platform: NodeJS.Platform) => readonly string[];
  readonly DIST_DIR: string;
  readonly cargoTargetDir: (platform: NodeJS.Platform) => string;
}

interface Preflight {
  readonly REMEDY: Readonly<Record<string, string>>;
  readonly checkNativeBindings: (
    root: string,
    platform?: NodeJS.Platform,
    arch?: string,
  ) => readonly string[];
  readonly remedyFor: (problem: string) => string | null;
}

async function load<T>(relative: string, names: readonly string[]): Promise<T> {
  const module: unknown = await import(/* @vite-ignore */ pathToFileURL(join(REPO, relative)).href);
  if (typeof module !== 'object' || module === null) {
    throw new Error(`${relative} did not export an object`);
  }
  for (const name of names) {
    if (!(name in module)) throw new Error(`${relative} exports no ${name}`);
  }
  return module as T;
}

const buildDist = await load<BuildDist>('scripts/build-dist.mjs', [
  'targetsFor',
  'DIST_DIR',
  'cargoTargetDir',
]);
const preflight = await load<Preflight>('scripts/preflight.mjs', [
  'REMEDY',
  'checkNativeBindings',
  'remedyFor',
]);

describe('targetsFor', () => {
  it("is the host's targets and nothing else", () => {
    expect(buildDist.targetsFor('win32')).toEqual(['nsis', 'portable']);
    expect(buildDist.targetsFor('linux')).toEqual(['AppImage', 'deb', 'rpm']);
  });

  it('refuses a platform this project does not ship rather than returning nothing to do', () => {
    expect(() => buildDist.targetsFor('darwin')).toThrow(/darwin/u);
  });

  it('writes beside the bundler output, not into it', () => {
    expect(buildDist.DIST_DIR).toBe('dist');
    expect(buildDist.DIST_DIR).not.toBe('out');
  });

  it('keeps the two toolchains in separate cargo target directories', () => {
    // They collide in one directory and the symptom is a link error naming an untouched
    // crate, not a rebuild.
    expect(buildDist.cargoTargetDir('win32')).not.toBe(buildDist.cargoTargetDir('linux'));
  });
});

describe('the preflight remedies', () => {
  it('every remedy names a command the reader can run', () => {
    const remedies = Object.values(preflight.REMEDY);
    expect(remedies.length).toBeGreaterThan(0);
    for (const remedy of remedies) expect(remedy).toMatch(/npm |cargo |node /u);
  });

  it('every problem string it can produce maps back to a remedy', () => {
    // A problem whose key does not exist in REMEDY would print the symptom and no fix, which
    // is the state this module was written to end.
    const problems = preflight.checkNativeBindings(REPO, 'darwin', 'x64');
    expect(problems.length).toBeGreaterThan(0);
    for (const problem of problems) expect(preflight.remedyFor(problem)).not.toBeNull();
  });

  it('reports this platform as ready in a tree the gates are already running in', () => {
    expect(preflight.checkNativeBindings(REPO)).toEqual([]);
  });
});

describe('the two wrappers', () => {
  it('delegate rather than reimplement, so the platforms cannot drift', () => {
    for (const wrapper of ['scripts/build-dist.sh', 'scripts/build-dist.cmd']) {
      const text = readFileSync(join(REPO, wrapper), 'utf8');
      expect(text, wrapper).toContain('build-dist.mjs');
      expect(text, wrapper).not.toMatch(/electron-builder|cargo build/u);
    }
  });
});

describe('the packaging config', () => {
  const config = readFileSync(join(REPO, 'electron-builder.yml'), 'utf8');

  it('carries both Windows targets', () => {
    // Read as text rather than parsed: no YAML parser is a declared dependency, and the two
    // target names are what this asserts.
    expect(config).toMatch(/target:\s*nsis/u);
    expect(config).toMatch(/target:\s*portable/u);
  });

  it('publishes nowhere', () => {
    expect(config).toMatch(/^publish: null$/mu);
  });

  it('writes its output to dist', () => {
    expect(config).toMatch(/output:\s*dist\b/u);
  });
});

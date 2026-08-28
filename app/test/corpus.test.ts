import * as fs from 'node:fs';
import { beforeAll, describe, expect, it } from 'vitest';

import type { CorpusManifest } from './corpus';
import {
  CORPUS_VERSION,
  FIXTURES,
  ensureCorpus,
  parseCorpusManifest,
  requireFixture,
} from './corpus';

describe('the corpus manifest contract', () => {
  it('rejects a manifest from a different corpus version', () => {
    expect(() =>
      parseCorpusManifest({
        corpusVersion: 999,
        gitVersion: '2.43.0',
        root: '/x',
        volumes: [],
        fixtures: [],
      }),
    ).toThrow(/corpusVersion/);
  });

  it('names the key that is missing rather than returning undefined', () => {
    expect(() => parseCorpusManifest({ corpusVersion: CORPUS_VERSION, root: '/x' })).toThrow(
      /gitVersion/,
    );
  });

  it('rejects a fixture whose expect block has the wrong shape', () => {
    expect(() =>
      parseCorpusManifest({
        corpusVersion: CORPUS_VERSION,
        gitVersion: '2.43.0',
        root: '/x',
        volumes: [],
        fixtures: [
          { name: 'a', volume: 'vol-a', path: '/x/a', materialised: true, skipReason: null },
        ],
      }),
    ).toThrow(/fixtures\[0\]\.expect/);
  });
});

describe('the generated corpus', () => {
  // Built in a hook, not at collection time: the first run compiles and runs the Rust
  // generator, and only a hook has a timeout that covers it.
  let manifest: CorpusManifest;
  beforeAll(() => {
    manifest = ensureCorpus();
  }, 600_000);

  it('matches the version this file was written against', () => {
    expect(manifest.corpusVersion).toBe(CORPUS_VERSION);
  });

  it('contains every fixture id this file knows about', () => {
    const present = new Set(manifest.fixtures.map((f) => f.name));
    for (const id of Object.values(FIXTURES)) {
      expect(present.has(id), `${id} is missing from the manifest`).toBe(true);
    }
  });

  it('hands back real paths on disk', () => {
    const bare = requireFixture(manifest, FIXTURES.bare);
    expect(fs.existsSync(bare.path)).toBe(true);
    expect(bare.expect.bare).toBe(true);
  });

  it('refuses a fixture the platform skipped instead of pretending it passed', () => {
    const skipped = manifest.fixtures.filter((f) => !f.materialised);
    for (const fixture of skipped) {
      expect(fixture.skipReason).toBeTruthy();
      expect(() => requireFixture(manifest, fixture.name)).toThrow(/unavailable/);
    }
  });
});

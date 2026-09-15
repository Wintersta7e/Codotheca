import * as path from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import {
  ART_IMMUTABLE_CACHE,
  ART_MEDIA_TYPE,
  type ArtProtocolDeps,
  createArtProtocolHandler,
  parseArtAddress,
  registerArtProtocol,
  renditionFilePath,
} from '../src/main/art/artProtocol';
import { artUrl } from '../src/shared/artAddress';
import type { SceneHash } from '../src/generated/protocol';

const HASH = '0123456789abcdef'.repeat(4);
const DATA_DIR = path.join('/data', 'codotheca');

function deps(overrides: Partial<ArtProtocolDeps> = {}): ArtProtocolDeps {
  return {
    dataDir: DATA_DIR,
    readRendition: () => Promise.resolve(Buffer.from([0x52, 0x49, 0x46, 0x46])),
    log: { write: vi.fn() },
    ...overrides,
  };
}

describe('parseArtAddress', () => {
  it('takes exactly two segments after the host', () => {
    expect(parseArtAddress(`codotheca://art/${HASH}/card`)).toEqual({
      hash: HASH,
      rendition: 'card',
    });
    expect(parseArtAddress(`codotheca://art/${HASH}/hero`)).toEqual({
      hash: HASH,
      rendition: 'hero',
    });
    // §7.6, criterion 62: one filename cannot hold both renditions, so a one-segment fetch
    // must not resolve to either of them.
    expect(parseArtAddress(`codotheca://art/${HASH}`)).toBeNull();
    expect(parseArtAddress(`codotheca://art/${HASH}/`)).toBeNull();
    expect(parseArtAddress(`codotheca://art/${HASH}/card/extra`)).toBeNull();
    expect(parseArtAddress(`codotheca://art/${HASH}//card`)).toBeNull();
  });

  it('refuses another scheme, another host and another rendition', () => {
    expect(parseArtAddress(`file:///${HASH}/card`)).toBeNull();
    expect(parseArtAddress(`https://art/${HASH}/card`)).toBeNull();
    expect(parseArtAddress(`codotheca://scenes/${HASH}/card`)).toBeNull();
    expect(parseArtAddress(`codotheca://art/${HASH}/thumbnail`)).toBeNull();
    expect(parseArtAddress('not a url at all')).toBeNull();
  });

  it('refuses anything that is not sixty-four lowercase hex', () => {
    expect(parseArtAddress('codotheca://art/../card')).toBeNull();
    expect(parseArtAddress(`codotheca://art/${HASH.toUpperCase()}/card`)).toBeNull();
    expect(parseArtAddress(`codotheca://art/${HASH.slice(0, 63)}/card`)).toBeNull();
    expect(parseArtAddress(`codotheca://art/${HASH}0/card`)).toBeNull();
    expect(parseArtAddress('codotheca://art/%2e%2e%2fetc%2fpasswd/card')).toBeNull();
  });
});

describe('renditionFilePath', () => {
  it('is the two-level fan-out the core writes, byte for byte', () => {
    // Ruling 4: this literal is mirrored in core/src/art/mod.rs::rendition_path and is asserted
    // there against the same string. Change one and the other's test fails.
    expect(renditionFilePath(DATA_DIR, { hash: HASH, rendition: 'card' })).toBe(
      path.join(DATA_DIR, 'art', '01', `${HASH}.card.webp`),
    );
    expect(renditionFilePath(DATA_DIR, { hash: HASH, rendition: 'hero' })).toBe(
      path.join(DATA_DIR, 'art', '01', `${HASH}.hero.webp`),
    );
  });

  it('never resolves outside the rendition store', () => {
    expect(renditionFilePath(DATA_DIR, { hash: '..', rendition: 'card' })).toBeNull();
    expect(renditionFilePath(DATA_DIR, { hash: `../../${HASH}`, rendition: 'card' })).toBeNull();
  });
});

describe('the handler', () => {
  it('serves the bytes as an immutable webp', async () => {
    const bytes = Buffer.from([1, 2, 3, 4, 5]);
    const handler = createArtProtocolHandler(deps({ readRendition: () => Promise.resolve(bytes) }));
    const res = await handler({ url: `codotheca://art/${HASH}/card` });

    expect(res.status).toBe(200);
    expect(res.headers.get('Content-Type')).toBe(ART_MEDIA_TYPE);
    expect(res.headers.get('Cache-Control')).toBe(ART_IMMUTABLE_CACHE);
    expect(Buffer.from(await res.arrayBuffer())).toEqual(bytes);
  });

  it('reads the file the address names and no other', async () => {
    const readRendition = vi.fn(() => Promise.resolve(Buffer.alloc(0)));
    const handler = createArtProtocolHandler(deps({ readRendition }));
    await handler({ url: `codotheca://art/${HASH}/hero` });
    expect(readRendition).toHaveBeenCalledWith(
      path.join(DATA_DIR, 'art', '01', `${HASH}.hero.webp`),
    );
  });

  it('refuses a malformed address with 400 and reads nothing', async () => {
    const readRendition = vi.fn(() => Promise.resolve(Buffer.alloc(0)));
    const handler = createArtProtocolHandler(deps({ readRendition }));
    const res = await handler({ url: `codotheca://art/${HASH}` });
    expect(res.status).toBe(400);
    expect(readRendition).not.toHaveBeenCalled();
  });

  it('answers 404 for a swept file and leaks no path in either direction', async () => {
    const missing = Object.assign(new Error('nope'), { code: 'ENOENT' });
    const write = vi.fn();
    const handler = createArtProtocolHandler(
      deps({ readRendition: () => Promise.reject(missing), log: { write } }),
    );
    const res = await handler({ url: `codotheca://art/${HASH}/card` });

    expect(res.status).toBe(404);
    expect(await res.text()).toBe('');
    // §7.5 demotes a missing bitmap to the nameplate; that is the renderer's business and needs
    // no diagnostics. A 404 is expected traffic, not a fault, so nothing is logged.
    expect(write).not.toHaveBeenCalled();
  });

  it('answers 500 for an unreadable file and logs the address, never the file', async () => {
    const denied = Object.assign(new Error('denied'), { code: 'EACCES' });
    const write = vi.fn();
    const handler = createArtProtocolHandler(
      deps({ readRendition: () => Promise.reject(denied), log: { write } }),
    );
    const res = await handler({ url: `codotheca://art/${HASH}/card` });

    expect(res.status).toBe(500);
    expect(await res.text()).toBe('');
    expect(write).toHaveBeenCalledTimes(1);
    const line = String(write.mock.calls[0]?.[2] ?? '');
    expect(line).toContain(HASH);
    expect(line).not.toContain(DATA_DIR);
  });
});

describe('registration', () => {
  it('binds the scheme the privileges were declared for', () => {
    const handle = vi.fn();
    registerArtProtocol({ handle }, deps());
    expect(handle).toHaveBeenCalledTimes(1);
    expect(handle.mock.calls[0]?.[0]).toBe('codotheca');
  });
});

/**
 * R47's shell half. `isRendition` is an array-membership test, so it admits the hyphen the moment
 * `RENDITIONS` carries it — but the two-segment rule is decided by the `/` split and must still
 * hold, and the filename literal is pinned on both sides.
 */
describe('§23.5: the two blueprint addresses', () => {
  it('parses a hyphenated slug as one segment', () => {
    expect(parseArtAddress(`codotheca://art/${HASH}/card-blueprint`)).toEqual({
      hash: HASH,
      rendition: 'card-blueprint',
    });
    expect(parseArtAddress(`codotheca://art/${HASH}/hero-blueprint`)).toEqual({
      hash: HASH,
      rendition: 'hero-blueprint',
    });
  });

  it('still refuses a trailing slash, a doubled slash and a third segment', () => {
    expect(parseArtAddress(`codotheca://art/${HASH}/card-blueprint/`)).toBeNull();
    expect(parseArtAddress(`codotheca://art/${HASH}//card-blueprint`)).toBeNull();
    expect(parseArtAddress(`codotheca://art/${HASH}/card-blueprint/extra`)).toBeNull();
    expect(parseArtAddress(`codotheca://art/${HASH}/card/blueprint`)).toBeNull();
    // A hyphen does not make every slug legal.
    expect(parseArtAddress(`codotheca://art/${HASH}/card-thumbnail`)).toBeNull();
  });

  it('builds the filename the core writes, pinned on both sides', () => {
    expect(renditionFilePath(DATA_DIR, { hash: HASH, rendition: 'card-blueprint' })).toBe(
      path.join(DATA_DIR, 'art', HASH.slice(0, 2), `${HASH}.card-blueprint.webp`),
    );
    expect(renditionFilePath(DATA_DIR, { hash: HASH, rendition: 'hero-blueprint' })).toBe(
      path.join(DATA_DIR, 'art', HASH.slice(0, 2), `${HASH}.hero-blueprint.webp`),
    );
  });

  it('gives the card and its blueprint different addresses over one hash', () => {
    // The hazard §23.5 names: without a distinct address a cached raster of one pass would be
    // served for the other at exactly the moment the project changes state.
    const card = artUrl(HASH as SceneHash, 'card');
    const blueprint = artUrl(HASH as SceneHash, 'card-blueprint');
    expect(card).not.toBe(blueprint);
    expect(blueprint).toBe(`codotheca://art/${HASH}/card-blueprint`);
    expect(artUrl(HASH as SceneHash, 'hero')).not.toBe(artUrl(HASH as SceneHash, 'hero-blueprint'));
  });
});

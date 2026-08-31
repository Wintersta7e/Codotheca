/**
 * `codotheca://art/<hash>/<rendition>` (§7.2, §7.6).
 *
 * With `sandbox: true` and no Node integration the renderer cannot load `file:`, so this is the
 * only route by which card art reaches the screen — and it is a one-way door. The renderer names
 * a **content address**, never a location; this module is the only code that turns one into a
 * path, and no path travels back out, not in a body and not in a log line.
 *
 * The `<data_dir>/art/<aa>/<hash>.<rendition>.webp` grammar is mirrored from
 * `core/src/art/mod.rs::rendition_path`, deliberately: the alternative is ~27 MB of card
 * renditions through a transport whose frame cap is 8 MB. Both sides pin the same literal in a
 * test, so a change to either fails the other.
 */
import { readFile } from 'node:fs/promises';
import * as path from 'node:path';

import type { Rendition } from '../../generated/protocol';
import type { RollingLog } from '../core/log';
import { ART_SCHEME, ART_SCHEME_HOST } from '../scheme';

export const ART_MEDIA_TYPE = 'image/webp';

/** The address is the SHA-256 of the scene, so the bytes behind it can never change. */
export const ART_IMMUTABLE_CACHE = 'public, max-age=31536000, immutable';

/** 64 lowercase hex and nothing else — the same total guard as `core::art::is_scene_hash`. */
export const SCENE_HASH_PATTERN = /^[0-9a-f]{64}$/u;

const RENDITIONS: readonly Rendition[] = ['card', 'hero'];

export interface ArtAddress {
  readonly hash: string;
  readonly rendition: Rendition;
}

/** Electron passes a whole `Request`; only its url is ever read, and a test needs no more. */
export interface ArtRequestLike {
  readonly url: string;
}

export interface ArtProtocolDeps {
  readonly dataDir: string;
  readonly readRendition: (file: string) => Promise<Buffer>;
  readonly log: Pick<RollingLog, 'write'>;
}

export interface ArtProtocolHost {
  handle(scheme: string, handler: (request: ArtRequestLike) => Promise<Response>): void;
}

function isRendition(value: string): value is Rendition {
  return (RENDITIONS as readonly string[]).includes(value);
}

function isMissingFile(err: unknown): boolean {
  return (
    typeof err === 'object' &&
    err !== null &&
    'code' in err &&
    (err as { code?: unknown }).code === 'ENOENT'
  );
}

/** A bodyless refusal. Nothing about the filesystem crosses back to the renderer. */
function refuse(status: number): Response {
  return new Response(null, { status });
}

export function parseArtAddress(rawUrl: string): ArtAddress | null {
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    return null;
  }
  if (url.protocol !== `${ART_SCHEME}:` || url.host !== ART_SCHEME_HOST) return null;

  // `pathname` always begins with '/', so a two-segment address splits into exactly three parts
  // with an empty first. This rejects a trailing slash, a doubled slash and a third segment.
  const segments = url.pathname.split('/');
  if (segments.length !== 3 || segments[0] !== '') return null;

  const hash = segments[1];
  const slug = segments[2];
  if (hash === undefined || slug === undefined) return null;
  if (!SCENE_HASH_PATTERN.test(hash) || !isRendition(slug)) return null;

  return { hash, rendition: slug };
}

export function renditionFilePath(dataDir: string, address: ArtAddress): string | null {
  if (!SCENE_HASH_PATTERN.test(address.hash)) return null;

  // `join`, not `resolve`: on win32 `resolve` prepends the current drive, and the core writes
  // the path the caller gave it. The containment check below resolves both sides together, so
  // it still compares absolute forms.
  const root = path.join(dataDir, 'art');
  const file = path.join(
    root,
    address.hash.slice(0, 2),
    `${address.hash}.${address.rendition}.webp`,
  );
  // Redundant against the pattern above, and kept: containment is then a property of the code
  // rather than an inference about a regex.
  return path.resolve(file).startsWith(path.resolve(root) + path.sep) ? file : null;
}

export function createArtProtocolHandler(
  deps: ArtProtocolDeps,
): (request: ArtRequestLike) => Promise<Response> {
  return async (request: ArtRequestLike): Promise<Response> => {
    const address = parseArtAddress(request.url);
    if (address === null) return refuse(400);

    const file = renditionFilePath(deps.dataDir, address);
    if (file === null) return refuse(400);

    try {
      const bytes = await deps.readRendition(file);
      return new Response(bytes, {
        status: 200,
        headers: {
          'Content-Type': ART_MEDIA_TYPE,
          'Content-Length': String(bytes.byteLength),
          'Cache-Control': ART_IMMUTABLE_CACHE,
        },
      });
    } catch (err: unknown) {
      // §7.5: a swept or not-yet-rendered file is ordinary traffic. The renderer falls back to
      // the nameplate and `art.url` marks the row stale; there is no fault to report.
      if (isMissingFile(err)) return refuse(404);
      deps.log.write(
        'warn',
        'shell',
        `art rendition unreadable: ${address.hash}.${address.rendition}`,
      );
      return refuse(500);
    }
  };
}

/**
 * Called **after** `app.ready`. The scheme's privileges were declared before it, by plan 01's
 * `bootstrap()`; Electron throws at both ends of that window, so neither call sits inline in
 * `index.ts` where the order would be invisible.
 */
export function registerArtProtocol(host: ArtProtocolHost, deps: ArtProtocolDeps): void {
  host.handle(ART_SCHEME, createArtProtocolHandler(deps));
}

/** The real reader, so `index.ts` names `node:fs` once and the handler stays injectable. */
export function readRenditionFromDisk(file: string): Promise<Buffer> {
  return readFile(file);
}

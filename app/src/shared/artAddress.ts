import type { SceneHash } from '../generated/protocol';

/**
 * `codotheca://art/<hash>/<rendition>` (§7.2, §7.6). The scheme is **two-segment**: a fetch of
 * `codotheca://art/<hash>` fails, which is criterion 62's assertion and the reason the rendition
 * is part of the address rather than part of the filename alone.
 *
 * It lives in `shared/` because both sides need it and neither may reach the other: the main
 * process declares the scheme's privileges from these two constants before `app.ready`, and the
 * renderer composes addresses from them — but `tsconfig.web.json` deliberately excludes
 * `src/main/**` so the renderer is structurally unable to name a Node API.
 */
export const ART_SCHEME = 'codotheca';

/** The single host segment. */
export const ART_SCHEME_HOST = 'art';

export function artUrl(sceneHash: SceneHash | null, rendition: 'card' | 'hero'): string | null {
  if (sceneHash === null || sceneHash === '') return null;
  return `${ART_SCHEME}://${ART_SCHEME_HOST}/${sceneHash}/${rendition}`;
}

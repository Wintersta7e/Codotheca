import type { CustomScheme } from 'electron';
import { ART_SCHEME } from '../shared/artAddress';

/**
 * `codotheca://art/<hash>/<rendition>` (§7.2, §7.6).
 *
 * With `sandbox: true` and no Node integration the renderer cannot load `file:`, so this is
 * the only route by which card art reaches the screen. The privileges must be declared
 * before `app.ready` — Electron throws otherwise, and the throw would happen on a user's
 * machine.
 *
 * The two segments are `shared/artAddress.ts`'s, which is also where the renderer composes
 * addresses from them. The renderer cannot import this file — `tsconfig.web.json` excludes
 * `src/main/**` — so a copy here would be the scheme spelled twice for the two ends of one URL.
 */
export { ART_SCHEME, ART_SCHEME_HOST } from '../shared/artAddress';

export const ART_SCHEME_PRIVILEGES: CustomScheme = {
  scheme: ART_SCHEME,
  privileges: {
    // A hierarchical URL, so `new URL()` gives a host and a path rather than an opaque blob.
    standard: true,
    // A secure context, so the renderer is not downgraded by loading art.
    secure: true,
    supportFetchAPI: true,
    stream: true,
    // fetch() from the renderer's own origin crosses an origin boundary into this scheme.
    corsEnabled: true,
    // The policy already names codotheca: for img-src and connect-src. Nothing needs a bypass.
    bypassCSP: false,
  },
};

export function registerArtSchemePrivileges(register: (schemes: CustomScheme[]) => void): void {
  register([ART_SCHEME_PRIVILEGES]);
}

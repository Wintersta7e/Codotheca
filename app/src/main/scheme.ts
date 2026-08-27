import type { CustomScheme } from 'electron';

/**
 * `codotheca://art/<hash>/<rendition>` (§7.2, §7.6).
 *
 * With `sandbox: true` and no Node integration the renderer cannot load `file:`, so this is
 * the only route by which card art reaches the screen. The privileges must be declared
 * before `app.ready` — Electron throws otherwise, and the throw would happen on a user's
 * machine.
 */
export const ART_SCHEME = 'codotheca';

/** The single host segment. §7.6's address is two-segment: a fetch of `//art/<hash>` fails. */
export const ART_SCHEME_HOST = 'art';

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

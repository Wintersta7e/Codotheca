/**
 * Main-process security wiring. Every export is a plain function over plain data so it can be
 * tested without launching Electron; `index.ts` hands them to the real session.
 */

export interface HeadersReceivedDetails {
  readonly responseHeaders?: Record<string, string[]>;
}

export type HeadersReceivedCallback = (response: {
  responseHeaders: Record<string, string[]>;
}) => void;

/**
 * Stamps `policy` onto every response, replacing whatever was there. The `<meta>` tag in the
 * built HTML says the same thing; this is the second lock, and it covers responses the
 * document does not control.
 */
export function contentSecurityPolicyListener(
  policy: string,
): (details: HeadersReceivedDetails, callback: HeadersReceivedCallback) => void {
  return (details, callback) => {
    const headers: Record<string, string[]> = { ...(details.responseHeaders ?? {}) };
    delete headers['Content-Security-Policy'];
    delete headers['content-security-policy'];
    headers['Content-Security-Policy'] = [policy];
    callback({ responseHeaders: headers });
  };
}

/**
 * Phase 1 asks the OS for nothing: no camera, no microphone, no location, no notifications,
 * no external opener. The renderer has no code that requests any of them, so a request
 * arriving is a defect and denying it silently is the correct outcome.
 */
export function denyPermissionRequest(
  _webContents: unknown,
  _permission: string,
  callback: (granted: boolean) => void,
): void {
  callback(false);
}

/**
 * The window loads exactly one document and never leaves it. A fragment change is the same
 * document; anything else — including the art scheme, which is only ever a subresource — is a
 * navigation the renderer must not be able to perform.
 *
 * [p2] §25.5: the caller binds this to **both** `will-navigate` and `will-frame-navigate`. The
 * first fires for the main frame only, and the README panel is the app's first subframe, so one
 * binding alone would have made the sentence above true of the window and false of what is in it.
 * One predicate, two events: a subframe may go exactly where the main frame may.
 *
 * **The README panel depends on this predicate never being consulted for `about:srcdoc`, and that
 * is a measurement, not a guarantee.** It refuses `about:srcdoc` — AC-P2-25-15 requires it — and
 * the handler cancels what it refuses. Measured on Electron 44.4.1: a sandboxed `srcdoc` frame
 * fires `will-frame-navigate` **zero** times, for both its initial commit and a `srcdoc`
 * reassignment, with a control proving the listener could see a same-origin subframe navigation.
 * If a future Chromium starts firing it, this predicate answers `false`, the commit is cancelled,
 * and **the panel renders an empty frame with no error anywhere** — the silent-nothing failure
 * `shared/csp.ts` widened `frame-src` expressly to avoid. The gate that would catch it is
 * `app/e2e/readme-census.spec.ts`, which asserts the frame rendered block elements. **On an
 * Electron upgrade, run it.**
 */
export function isNavigationAllowed(entryUrl: string, targetUrl: string): boolean {
  let entry: URL;
  let target: URL;
  try {
    entry = new URL(entryUrl);
    target = new URL(targetUrl);
  } catch {
    return false;
  }
  return entry.origin === target.origin && entry.pathname === target.pathname;
}

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

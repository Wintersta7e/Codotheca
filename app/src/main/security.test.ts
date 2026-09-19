import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
import { CONTENT_SECURITY_POLICY } from '../shared/csp';
import {
  contentSecurityPolicyListener,
  denyPermissionRequest,
  isNavigationAllowed,
} from './security';

describe('contentSecurityPolicyListener', () => {
  it('overwrites any policy the response already carried', () => {
    const listener = contentSecurityPolicyListener(CONTENT_SECURITY_POLICY);
    const callback = vi.fn();
    listener(
      { responseHeaders: { 'Content-Security-Policy': ['default-src *'], 'X-Keep': ['yes'] } },
      callback,
    );
    expect(callback).toHaveBeenCalledWith({
      responseHeaders: {
        'X-Keep': ['yes'],
        'Content-Security-Policy': [CONTENT_SECURITY_POLICY],
      },
    });
  });

  it('works when the response carried no headers at all', () => {
    const listener = contentSecurityPolicyListener("default-src 'none'");
    const callback = vi.fn();
    listener({}, callback);
    expect(callback).toHaveBeenCalledWith({
      responseHeaders: { 'Content-Security-Policy': ["default-src 'none'"] },
    });
  });
});

describe('denyPermissionRequest', () => {
  it('denies every permission', () => {
    // [p2] §25.2: `openExternal` stays denied after the external opener ships. That handler
    // grants for a whole session, and the opener is a channel the shell owns rather than a
    // capability the renderer holds. The count is printed because a loop over nothing denies
    // nothing and reads exactly like a loop that denied everything.
    //
    // [p3] §32.12: **`notifications` stays denied after the advisory alert ships.** The shell
    // posts it from the main process; the renderer originates none, which
    // `scripts/check-notification-origin.mjs` scans for. Granting it here would make the denial
    // and the scanner two answers to one question.
    const permissions = ['media', 'geolocation', 'notifications', 'openExternal'];
    process.stderr.write(
      `security: permission loop covered ${String(permissions.length)} name(s)\n`,
    );
    expect(permissions.length, 'the permission loop covered no name').toBeGreaterThan(0);
    for (const permission of permissions) {
      const callback = vi.fn();
      denyPermissionRequest(null, permission, callback);
      expect(callback, permission).toHaveBeenCalledWith(false);
    }
  });
});

describe('isNavigationAllowed', () => {
  const entry = 'file:///app/out/renderer/index.html';

  it('allows a navigation to the document the window was opened with', () => {
    expect(isNavigationAllowed(entry, entry)).toBe(true);
    expect(isNavigationAllowed(entry, `${entry}#project/7`)).toBe(true);
  });

  it('refuses anything else', () => {
    expect(isNavigationAllowed(entry, 'https://example.invalid')).toBe(false);
    expect(isNavigationAllowed(entry, 'file:///etc/passwd')).toBe(false);
    expect(isNavigationAllowed(entry, 'codotheca://art/abc/card')).toBe(false);
  });

  /**
   * [p2] §25.5 registers `will-frame-navigate` beside `will-navigate` with the **same**
   * predicate, because `will-navigate` fires for the main frame only and §25.5 adds the app's
   * first subframe.
   *
   * **Branch A, measured** (`app/e2e/frame-nav-probe.spec.ts`, Electron 44.4.1, 2026-09-16, now
   * deleted): a sandboxed `srcdoc` frame produces **zero** `will-frame-navigate` events — none
   * for the initial `about:srcdoc` commit and none for a `srcdoc` reassignment — while the same
   * listener saw exactly one event for a same-origin subframe `src`, which is what proves the
   * instrument could see one at all. So the predicate is bound verbatim, it stays
   * byte-identical, and there is no second export.
   */
  it('security::ac_p2_25_15_will_navigate_and_will_frame_navigate_share_one_predicate', () => {
    expect(isNavigationAllowed(entry, 'about:srcdoc')).toBe(false);
    expect(isNavigationAllowed(entry, 'https://example.com/')).toBe(false);
    expect(isNavigationAllowed(entry, 'data:text/html,<script>alert(1)</script>')).toBe(false);

    // Both guards are bound on the window's webContents, read off the source that binds them:
    // a predicate that refuses everything proves nothing if nothing calls it.
    const main = readFileSync(fileURLToPath(new URL('./index.ts', import.meta.url)), 'utf8');
    expect(main.length, 'read real source, or the assertions below are vacuous').toBeGreaterThan(
      2000,
    );
    const bindings = [...main.matchAll(/webContents\.on\('(will-navigate|will-frame-navigate)'/gu)]
      .map((match) => match[1])
      .sort();
    process.stderr.write(`security: navigation guards bound: ${bindings.join(', ')}\n`);
    expect(bindings).toEqual(['will-frame-navigate', 'will-navigate']);
    // …and both of them through this one predicate, not two.
    expect(main.match(/isNavigationAllowed\(entry, /gu)?.length).toBe(2);
  });
});

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
});

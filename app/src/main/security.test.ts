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
    for (const permission of ['media', 'geolocation', 'notifications', 'openExternal']) {
      const callback = vi.fn();
      denyPermissionRequest(null, permission, callback);
      expect(callback).toHaveBeenCalledWith(false);
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

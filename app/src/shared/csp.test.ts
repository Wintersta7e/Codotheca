import { describe, expect, it } from 'vitest';
import { CONTENT_SECURITY_POLICY, developmentContentSecurityPolicy, remoteOriginsIn } from './csp';

describe('the production content security policy', () => {
  it('names no remote origin at all', () => {
    // No account, no telemetry is a product guarantee. A policy that would admit a CDN, a
    // font host or an analytics endpoint makes the footer's claim false.
    expect(remoteOriginsIn(CONTENT_SECURITY_POLICY)).toEqual([]);
  });

  it('starts from default-src none and self-hosts fonts', () => {
    expect(CONTENT_SECURITY_POLICY).toContain("default-src 'none'");
    expect(CONTENT_SECURITY_POLICY).toContain("font-src 'self'");
  });

  it('admits the art scheme for images and for fetch', () => {
    expect(CONTENT_SECURITY_POLICY).toContain('img-src');
    expect(CONTENT_SECURITY_POLICY).toMatch(/img-src[^;]*codotheca:/);
    expect(CONTENT_SECURITY_POLICY).toMatch(/connect-src[^;]*codotheca:/);
  });
});

describe('the development content security policy', () => {
  it('adds the loopback dev server and its websocket', () => {
    const policy = developmentContentSecurityPolicy('http://localhost:5173');
    expect(policy).toContain('http://localhost:5173');
    expect(policy).toContain('ws://localhost:5173');
    expect(policy).toContain("default-src 'none'");
  });

  it('refuses an origin that is not loopback', () => {
    expect(() => developmentContentSecurityPolicy('https://example.invalid')).toThrow(
      /non-loopback/,
    );
  });
});

describe('remoteOriginsIn', () => {
  it('reports the offending source expressions and not the directive names', () => {
    expect(remoteOriginsIn("default-src 'none'; font-src https://fonts.example")).toEqual([
      'https://fonts.example',
    ]);
  });
});

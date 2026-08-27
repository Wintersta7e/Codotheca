import { describe, expect, it } from 'vitest';
import { ART_SCHEME, ART_SCHEME_HOST, ART_SCHEME_PRIVILEGES } from './scheme';

describe('the art scheme', () => {
  it('is addressed the way the spec writes it', () => {
    expect(ART_SCHEME).toBe('codotheca');
    expect(ART_SCHEME_HOST).toBe('art');
    expect(ART_SCHEME_PRIVILEGES.scheme).toBe('codotheca');
  });

  it('declares fetch and stream support, and does not bypass the policy', () => {
    // §7.2 names supportFetchAPI and stream. bypassCSP stays false: the policy already
    // admits codotheca: for img-src and connect-src, so a bypass would only widen the
    // scheme's reach past what was reasoned about.
    expect(ART_SCHEME_PRIVILEGES.privileges).toMatchObject({
      standard: true,
      secure: true,
      supportFetchAPI: true,
      stream: true,
      corsEnabled: true,
      bypassCSP: false,
    });
  });
});

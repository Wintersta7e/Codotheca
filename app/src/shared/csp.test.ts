import { describe, expect, it } from 'vitest';
import { CONTENT_SECURITY_POLICY, developmentContentSecurityPolicy, remoteOriginsIn } from './csp';

/**
 * The policy as a map, so a directive can be compared to its whole value.
 *
 * `toContain("font-src 'self'")` keeps passing against `font-src 'self' https://cdn.example`,
 * which is exactly the drift this file must not permit — and §25.5's one-directive change is the
 * moment that stops being theoretical.
 */
function directives(policy: string): Map<string, string[]> {
  const parsed = new Map<string, string[]>();
  for (const clause of policy.split(';')) {
    const tokens = clause
      .trim()
      .split(/\s+/u)
      .filter((token) => token.length > 0);
    const [name, ...sources] = tokens;
    if (name === undefined) continue;
    parsed.set(name, sources);
  }
  return parsed;
}

/** Phase 1's policy, byte for byte, with §25.5's one change. */
const EXPECTED: readonly (readonly [string, readonly string[]])[] = [
  ['default-src', ["'none'"]],
  ['script-src', ["'self'"]],
  ['style-src', ["'self'", "'unsafe-inline'"]],
  ['font-src', ["'self'"]],
  ['img-src', ["'self'", 'codotheca:', 'data:']],
  ['connect-src', ['codotheca:']],
  ['worker-src', ["'self'"]],
  ['media-src', ["'none'"]],
  ['object-src', ["'none'"]],
  // [p2] §25.5. The only change in the whole section.
  ['frame-src', ["'self'"]],
  ['child-src', ["'none'"]],
  ['manifest-src', ["'none'"]],
  ['base-uri', ["'none'"]],
  ['form-action', ["'none'"]],
];

describe('the production content security policy', () => {
  it('names no remote origin at all', () => {
    // No account, no telemetry is a product guarantee. A policy that would admit a CDN, a
    // font host or an analytics endpoint makes the footer's claim false. `'self'` is in
    // ALLOWED_SOURCES, so §25.5's frame-src change leaves this assertion's meaning intact.
    expect(remoteOriginsIn(CONTENT_SECURITY_POLICY)).toEqual([]);
  });

  it('csp::ac_p2_25_14_frame_src_self_and_every_other_directive_byte_identical', () => {
    const parsed = directives(CONTENT_SECURITY_POLICY);
    // The directive **set** first: a directive added later fails here rather than passing
    // unnoticed under an assertion that only looks at the ones it already knows about.
    expect([...parsed.keys()]).toEqual(EXPECTED.map(([name]) => name));
    for (const [name, sources] of EXPECTED) {
      expect(parsed.get(name), name).toEqual(sources);
    }
    expect(remoteOriginsIn(CONTENT_SECURITY_POLICY)).toEqual([]);
  });

  it('admits the art scheme for images and for fetch, and nothing else', () => {
    const parsed = directives(CONTENT_SECURITY_POLICY);
    expect(parsed.get('img-src')).toContain('codotheca:');
    expect(parsed.get('connect-src')).toEqual(['codotheca:']);
  });
});

describe('the development content security policy', () => {
  it('adds the loopback dev server and its websocket', () => {
    const policy = developmentContentSecurityPolicy('http://localhost:5173');
    expect(policy).toContain('http://localhost:5173');
    expect(policy).toContain('ws://localhost:5173');
    expect(policy).toContain("default-src 'none'");
  });

  it('leaves frame-src alone, so the dev path needs no new replacement', () => {
    // §25.5: `developmentContentSecurityPolicy` does not touch `frame-src`, and `'self'` is what
    // the frame needs in both modes. A `.replace()` added there would be a second owner for the
    // directive.
    const parsed = directives(developmentContentSecurityPolicy('http://localhost:5173'));
    expect(parsed.get('frame-src')).toEqual(["'self'"]);
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

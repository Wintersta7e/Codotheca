import { describe, expect, it } from 'vitest';

import { BODY_SIZES, DISPLAY_SIZES, MONO_SIZES } from '../../theme/type';
import {
  buildSrcdoc,
  FRAME_HEIGHT_PX,
  FRAME_TOKEN_NAMES,
  frameTokens,
  LINKS_INERT_NOTICE,
  readFrameTokens,
} from './frame';
import { renderMarkup } from './markup';

const VALUES = {
  '--surface-2': '#101418',
  '--surface-3': '#12161a',
  '--line-2': '#1f242a',
  '--text-1': '#dde3e8',
  '--text-2': '#b6c1cb',
  '--text-3': '#8b97a3',
} as const;
const TOKENS = frameTokens(VALUES);

function srcdocFor(source: string): string {
  return buildSrcdoc(renderMarkup(source).fragment, TOKENS);
}

describe('the frame document', () => {
  it('carries a charset, one stylesheet and the sanitised body', () => {
    const srcdoc = srcdocFor('# widget\n\nProse.\n');
    expect(srcdoc.startsWith('<!doctype html>')).toBe(true);
    expect(srcdoc).toContain('<meta charset="utf-8">');
    expect(srcdoc).toContain('<h1>widget</h1>');
    expect(srcdoc.match(/<style>/gu)).toHaveLength(1);
  });

  it('types no colour literal of its own and drops a token that is not a colour', () => {
    const srcdoc = srcdocFor('# widget\n');
    // Every hex colour in the document came from the injected token set, and nothing else.
    const hexes = [...srcdoc.matchAll(/#[0-9a-f]{3,8}\b/giu)].map((match) => match[0]);
    for (const hex of hexes) {
      expect(Object.values(VALUES), hex).toContain(hex);
    }

    const hostile = buildSrcdoc(
      renderMarkup('# widget\n').fragment,
      // `url(...)` in a custom property would be a request from a frame that makes none, and the
      // constructor is the only way a value reaches the document at all.
      frameTokens({
        '--text-1': 'url(https://cdn.example.test/x.png)',
        '--text-2': '#b6c1cb',
      }),
    );
    expect(hostile).not.toContain('cdn.example.test');
    expect(hostile).toContain('--text-2: #b6c1cb');
  });

  it('sizes every rule from §8.7 scale', () => {
    const srcdoc = srcdocFor('# widget\n');
    const scale = new Set([...DISPLAY_SIZES, ...BODY_SIZES, ...MONO_SIZES]);
    const sizes = [...srcdoc.matchAll(/font-size:\s*([0-9.]+)px/gu)].map((match) =>
      Number(match[1]),
    );
    expect(sizes.length, 'the frame declares at least one size').toBeGreaterThan(0);
    for (const size of sizes) {
      expect(scale.has(size), `${String(size)}px is off §8.7's scale`).toBe(true);
    }
  });

  it('renders an anchor with no link affordance', () => {
    const srcdoc = srcdocFor('[docs](https://example.test/docs)\n');
    expect(srcdoc).toContain('<a href="https://example.test/docs">docs</a>');
    // Read the rule out of the emitted stylesheet: colour inherited, no underline, no pointer.
    const rule = /a\s*\{([^}]*)\}/u.exec(srcdoc)?.[1] ?? '';
    expect(rule).toContain('color: inherit');
    expect(rule).toContain('text-decoration: none');
    expect(rule).toContain('cursor: default');
  });

  it('holds the panel to one fixed height, because a scriptless frame cannot report one', () => {
    expect(FRAME_HEIGHT_PX).toBe(420);
  });

  it('names the six tokens it injects and no others', () => {
    expect([...FRAME_TOKEN_NAMES]).toEqual([
      '--surface-2',
      '--surface-3',
      '--line-2',
      '--text-1',
      '--text-2',
      '--text-3',
    ]);
  });

  it('reads the tokens off a live element rather than restating them', () => {
    const host = document.createElement('div');
    host.style.setProperty('--text-1', '#dde3e8');
    host.style.setProperty('--line-2', 'not a colour');
    document.body.appendChild(host);
    try {
      const tokens = readFrameTokens(host);
      expect(tokens['--text-1']).toBe('#dde3e8');
      expect(tokens['--line-2']).toBeUndefined();
    } finally {
      host.remove();
    }
  });
});

describe('the anchors statement', () => {
  it('readmeFrame::ac_p2_25_22_the_links_are_not_active_statement_renders_once', () => {
    // The statement belongs to the panel, below the frame, and the frame itself never carries
    // it — so the count this criterion is about is the count of anchors the document has.
    const two = renderMarkup('[a](https://example.test/a) and [b](https://example.test/b)\n');
    expect(two.anchorCount).toBe(2);
    const none = renderMarkup('Just prose, no links.\n');
    expect(none.anchorCount).toBe(0);
    expect(LINKS_INERT_NOTICE).toBe('LINKS ARE NOT ACTIVE IN THIS PANEL');
    // Whatever the document says, the notice is not inside the frame: one statement, in the
    // panel's own voice, in the parent document where the panel can style it.
    expect(buildSrcdoc(two.fragment, TOKENS)).not.toContain(LINKS_INERT_NOTICE);
  });
});

/**
 * **This file does NOT carry AC-P2-25-16.** It asserts what the DOM does with an attribute, which
 * is a fact about the browser and not about the product: an earlier version of it claimed to
 * assert "the element the panel renders" while building its own `iframe`, so deleting
 * `sandbox=""` from `ReadmePanel.tsx` left every gate green. The criterion now lives on
 * `ReadmePanel.test.tsx`, which renders the panel and reads the attribute off the element React
 * produced, and on the census, which reads it off the real Chromium element.
 *
 * What is left here is the **serialised flag vocabulary**: whatever the sandbox attribute is, the
 * document it wraps must never name a flag that would undo it.
 */
describe('the sandbox vocabulary', () => {
  it('names no flag in the document the frame carries', () => {
    const srcdoc = srcdocFor('# widget\n\n[docs](https://example.test)\n');
    for (const flag of ['allow-scripts', 'allow-same-origin', 'allow-top-navigation']) {
      expect(srcdoc.includes(flag), flag).toBe(false);
    }
  });

  it('is exactly empty when the DOM is asked for it', () => {
    const frame = document.createElement('iframe');
    frame.setAttribute('sandbox', '');
    frame.setAttribute('srcdoc', srcdocFor('# widget\n'));
    expect(frame.hasAttribute('sandbox')).toBe(true);
    expect(frame.getAttribute('sandbox')).toBe('');
    // The attribute string itself, because that is what a browser parses and what the DOM
    // serialises. jsdom does not implement `HTMLIFrameElement.sandbox` as a DOMTokenList, and
    // asserting through a shim would be asserting the shim.
    const attribute = frame.getAttribute('sandbox') ?? 'missing';
    for (const flag of [
      'allow-scripts',
      'allow-same-origin',
      'allow-forms',
      'allow-popups',
      'allow-top-navigation',
      'allow-downloads',
    ]) {
      expect(attribute.includes(flag), flag).toBe(false);
    }
    expect(frame.outerHTML).toContain('sandbox=""');
  });
});

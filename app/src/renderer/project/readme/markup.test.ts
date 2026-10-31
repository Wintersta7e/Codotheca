import { describe, expect, it } from 'vitest';

import type { ReadmeAsset } from '../../../generated/protocol';
import { ALLOWED_TAGS, applyAssets, renderMarkup, serialiseFragment } from './markup';

/**
 * §25.10's AC-P2-25-17 document, verbatim in shape: the five things a README can carry that a
 * panel must not act on.
 *
 * D11 turned the parser's `html` on, so raw HTML reaches DOMPurify as nodes and the parser no
 * longer escapes anything: the explicit allowlist and `FORBID_CONTENTS` take the `<script>` away
 * with its text, and the `<details>` renders as the element it is. The scriptless `sandbox=""`
 * frame is the layer behind them (`readmeSandbox.test.ts`). Reverting the sanitiser turns the
 * script assertion red.
 */
const HOSTILE = [
  '# widget',
  '',
  '<script>alert(1)</script>',
  '',
  '<img src=x onerror="alert(2)">',
  '',
  '[x](javascript:alert(1))',
  '',
  '[y](data:text/html,<script>alert(3)</script>)',
  '',
  // THE DISCRIMINATING INPUT. `markdown-it`'s own `validateLink` already refuses `javascript:`
  // and `data:text/html`, so neither of those tells the two layers apart — the suite passed with
  // the sanitiser removed until this line existed. markdown-it **admits** a `data:image/*` href;
  // DOMPurify refuses `data:` in an `href` at all, which is where `data:text/html` in an href
  // stops being reachable by a later widening.
  '[z](data:image/png;base64,iVBORw0KGgo=)',
  '',
  '<details><summary>open</summary>hidden</details>',
  '',
  '![a badge](https://cdn.example.test/badge.svg)',
  '',
  '![a local diagram](docs/diagram.png)',
  '',
  '[a real link](https://example.test/docs) and <a href="javascript:alert(4)">raw</a>',
].join('\n');

function textOf(fragment: DocumentFragment): string {
  const host = document.createElement('div');
  host.appendChild(fragment.cloneNode(true));
  return host.textContent;
}

function elementsOf(fragment: DocumentFragment): Element[] {
  const host = document.createElement('div');
  host.appendChild(fragment.cloneNode(true));
  return [...host.querySelectorAll('*')];
}

describe('the README pipeline', () => {
  it('readmeMarkup::ac_p2_25_17_hostile_markup_leaves_no_script_handler_or_unsafe_href', () => {
    const rendered = renderMarkup(HOSTILE);
    const text = textOf(rendered.fragment);
    const elements = elementsOf(rendered.fragment);

    // 1. The script tag is gone, element and text: the sanitiser drops it with its contents.
    expect(elements.some((el) => el.tagName.toLowerCase() === 'script')).toBe(false);
    expect(text).not.toContain('alert(1)</script>');
    expect(text).not.toContain('<script>');

    // 2. No event handler survives anywhere in the fragment.
    for (const element of elements) {
      for (const attribute of element.getAttributeNames()) {
        expect(attribute.toLowerCase().startsWith('on'), `${element.tagName}[${attribute}]`).toBe(
          false,
        );
      }
    }

    // 3. No href survives whose scheme is not http(s) or mailto.
    const hrefs = elements
      .filter((el) => el.hasAttribute('href'))
      .map((el) => el.getAttribute('href') ?? '');
    expect(hrefs.length, 'the document carries at least one anchor to judge').toBeGreaterThan(0);
    // An allowlist read by the URL parser, the reader a click uses: a prefix check misses any
    // scheme it does not name, and `java\tscript:` or a leading space. A relative href resolves
    // against the https base and passes.
    for (const href of hrefs) {
      const { protocol } = new URL(href, 'https://base.test/');
      expect(['http:', 'https:', 'mailto:'], href).toContain(protocol);
    }

    // 4. Every image is a placeholder with no `src` attribute at all, so the first frame
    //    issues zero requests.
    const images = elements.filter((el) => el.tagName.toLowerCase() === 'img');
    expect(images).toHaveLength(0);
    for (const element of elements) {
      expect(element.hasAttribute('src'), element.tagName).toBe(false);
    }

    // 5. …and the references left in document order, remote before local as the document has
    //    them.
    //    The raw `<img src=x>` is one of them now that inline HTML parses.
    expect(rendered.imageRefs).toEqual([
      'x',
      'https://cdn.example.test/badge.svg',
      'docs/diagram.png',
    ]);
    expect(rendered.anchorCount).toBeGreaterThan(0);

    // 6. `<details>` is the element it is, not escaped text (D11).
    expect(elements.some((el) => el.tagName.toLowerCase() === 'details')).toBe(true);
    expect(text).not.toContain('<details>');
  });

  it('renders GFM tables and keeps the structure a reader needs', () => {
    const rendered = renderMarkup('| a | b |\n| - | - |\n| 1 | 2 |\n');
    const tags = elementsOf(rendered.fragment).map((el) => el.tagName.toLowerCase());
    expect(tags).toContain('table');
    expect(tags).toContain('td');
  });

  it('keeps a document with no images and no links honest about both', () => {
    const rendered = renderMarkup('# plain\n\nJust prose.\n');
    expect(rendered.imageRefs).toEqual([]);
    expect(rendered.anchorCount).toBe(0);
  });

  it('serialises what it sanitised, escaping the text it kept', () => {
    // A code span keeps the tag as text, which the serialiser must escape on the way out.
    const rendered = renderMarkup('`<script>alert(1)</script>`\n');
    const html = serialiseFragment(rendered.fragment);
    expect(html).toContain('&lt;script&gt;');
    expect(html).not.toContain('<script>');
  });
});

describe('applyAssets', () => {
  function assets(rows: Partial<ReadmeAsset>[]): ReadmeAsset[] {
    return rows.map((row) => ({
      ref: row.ref ?? '',
      state: row.state ?? 'ok',
      dataUri: row.dataUri ?? null,
      fetchedAt: row.fetchedAt ?? null,
    }));
  }

  it('puts an image back on the node the reference came from', () => {
    const rendered = renderMarkup('![alt text](docs/logo.png)\n');
    applyAssets(
      rendered.fragment,
      assets([
        {
          ref: 'docs/logo.png',
          state: 'ok',
          dataUri: 'data:image/png;base64,iVBORw0KGgo=',
        },
      ]),
    );
    const images = elementsOf(rendered.fragment).filter((el) => el.tagName.toLowerCase() === 'img');
    expect(images).toHaveLength(1);
    expect(images[0]?.getAttribute('src')).toBe('data:image/png;base64,iVBORw0KGgo=');
    expect(images[0]?.getAttribute('alt')).toBe('alt text');
  });

  it('refuses a data uri that is not one of the five image media types', () => {
    for (const hostile of [
      'data:text/html,<script>alert(1)</script>',
      'data:image/svg,<svg/>',
      'https://cdn.example.test/badge.svg',
      'javascript:alert(1)',
    ]) {
      const rendered = renderMarkup('![alt](docs/logo.png)\n');
      applyAssets(
        rendered.fragment,
        assets([{ ref: 'docs/logo.png', state: 'ok', dataUri: hostile }]),
      );
      const elements = elementsOf(rendered.fragment);
      expect(
        elements.some((el) => el.tagName.toLowerCase() === 'img'),
        hostile,
      ).toBe(false);
      for (const element of elements) {
        expect(element.hasAttribute('src'), hostile).toBe(false);
      }
    }
  });

  it('starts unasked, because a reference nobody asked about is not one consent refused', () => {
    const rendered = renderMarkup('![alt](https://cdn.example.test/badge.svg)\n');
    const placeholder = elementsOf(rendered.fragment).find((el) =>
      el.hasAttribute('data-asset-state'),
    );
    expect(placeholder?.getAttribute('data-asset-state')).toBe('unasked');
  });

  it('records a state that is not ok on the placeholder, and blocked is not a failure', () => {
    const rendered = renderMarkup('![alt](https://cdn.example.test/badge.svg)\n');
    applyAssets(
      rendered.fragment,
      assets([{ ref: 'https://cdn.example.test/badge.svg', state: 'blocked' }]),
    );
    const placeholder = elementsOf(rendered.fragment).find((el) =>
      el.hasAttribute('data-asset-state'),
    );
    expect(placeholder?.getAttribute('data-asset-state')).toBe('blocked');
    expect(placeholder?.textContent).toContain('alt');
  });
});

/**
 * D11: a README's inline HTML renders, and the frame still fetches nothing of its own.
 *
 * Every attribute below is one DOMPurify 3.4.17's **default** allowlist admits (`html`, `svg`,
 * `mathMl` and `xml` attribute lists in its dist) and whose value a browser can dereference: a
 * URL, a `srcset`, or `url(…)` in a style or an SVG paint. `cite` is in that list too and is left
 * out on purpose — it is a URL no user agent fetches.
 */
const NETWORK_ATTRS: readonly string[] = [
  'action',
  'background',
  'clip-path',
  'fill',
  'filter',
  'href',
  'marker-end',
  'marker-mid',
  'marker-start',
  'mask',
  'poster',
  'src',
  'srcset',
  'style',
  'xlink:href',
];

/** A value every one of them would fetch, if it survived. */
const PROBE_VALUE: Record<string, string> = {
  srcset: 'https://probe.invalid/a.png 2x',
  style: 'background-image:url(https://probe.invalid/a.png)',
};

function commentsOf(fragment: DocumentFragment): string[] {
  const host = document.createElement('div');
  host.appendChild(fragment.cloneNode(true));
  const walker = document.createTreeWalker(host, NodeFilter.SHOW_COMMENT);
  const found: string[] = [];
  while (walker.nextNode() !== null) found.push(walker.currentNode.textContent ?? '');
  return found;
}

describe('README inline HTML (D11)', () => {
  it('AC-P4-50-4 inline html renders with zero requests', () => {
    const source = [
      '# widget',
      '',
      '<p align="center"><img src="https://cdn.example.test/logo.png" srcset="https://cdn.example.test/logo@2x.png 2x" alt="the logo" width="120" height="40"></p>',
      '',
      '<!-- a maintainer note nobody should see -->',
      '',
      '<details><summary>More</summary>',
      '',
      'Collapsed body.',
      '',
      '</details>',
      '',
      '<picture><source media="(prefers-color-scheme: dark)" srcset="https://cdn.example.test/dark.png"><img src="https://cdn.example.test/light.png" alt="the mark"></picture>',
      '',
    ].join('\n');
    const rendered = renderMarkup(source);
    const elements = elementsOf(rendered.fragment);
    const tags = elements.map((el) => el.tagName.toLowerCase());

    // It rendered: the centred paragraph and the disclosure are elements, not escaped text.
    const centred = elements.find(
      (el) => el.tagName.toLowerCase() === 'p' && el.getAttribute('align') === 'center',
    );
    expect(centred, 'the centred paragraph is an element').toBeDefined();
    expect(tags).toContain('details');
    expect(tags).toContain('summary');
    expect(textOf(rendered.fragment)).not.toContain('<details>');

    // The image is the placeholder, in the image's place, and its reference waits for the core.
    const placeholder = centred?.querySelector('[data-readme-ref]');
    expect(placeholder?.getAttribute('data-readme-ref')).toBe('https://cdn.example.test/logo.png');
    expect(placeholder?.getAttribute('data-asset-state')).toBe('unasked');
    expect(placeholder?.textContent).toBe('the logo');
    expect(rendered.imageRefs).toEqual([
      'https://cdn.example.test/logo.png',
      'https://cdn.example.test/light.png',
    ]);

    // The comment renders as nothing at all.
    expect(textOf(rendered.fragment)).not.toContain('maintainer note');
    expect(commentsOf(rendered.fragment)).toEqual([]);

    // `<picture>` and `<source>` render nothing; the fallback `img` is walked like any other.
    expect(tags).not.toContain('picture');
    expect(tags).not.toContain('source');
    expect(tags).not.toContain('img');

    // Zero requests on the first frame: nothing the frame is handed can be dereferenced.
    for (const element of elements) {
      for (const attribute of NETWORK_ATTRS) {
        if (attribute === 'href' && element.tagName.toLowerCase() === 'a') continue;
        expect(element.hasAttribute(attribute), `${element.tagName}[${attribute}]`).toBe(false);
      }
    }
    const html = serialiseFragment(rendered.fragment);
    for (const attribute of NETWORK_ATTRS) {
      expect(html, attribute).not.toMatch(new RegExp(`\\s${attribute}=`, 'u'));
    }
  });

  it('AC-P4-50-4 every network-capable default attribute is absent or walked', () => {
    // MathML elements only parse as MathML inside `<math>`; table parts only inside a table.
    const MATHML = new Set(ALLOWED_TAGS.slice(ALLOWED_TAGS.indexOf('math') + 1));
    const TABLE = new Set(['thead', 'tbody', 'tfoot', 'tr', 'th', 'td', 'caption', 'colgroup']);
    const wrap = (tag: string, open: string): string => {
      if (MATHML.has(tag)) return `<math>${open}x</${tag}></math>`;
      if (TABLE.has(tag)) return `<table>${open}x</${tag}></table>`;
      return `${open}x</${tag}>`;
    };

    const TAGS = [...ALLOWED_TAGS, 'picture', 'source', 'video', 'audio', 'iframe', 'svg'];
    let cases = 0;
    let rendered = 0;
    const kept: string[] = [];
    // One document per attribute, one HTML block per tag: a render per case ran past the
    // suite's timeout under a full parallel run.
    for (const attribute of NETWORK_ATTRS) {
      const value = PROBE_VALUE[attribute] ?? 'https://probe.invalid/a.png';
      const source = TAGS.map(
        (tag) => `<div>${wrap(tag, `<${tag} ${attribute}="${value}">`)}</div>\n`,
      ).join('\n');
      const result = renderMarkup(source);
      const elements = elementsOf(result.fragment);
      const present = new Set(elements.map((el) => el.tagName.toLowerCase()));
      cases += TAGS.length;
      rendered += TAGS.filter((tag) => present.has(tag)).length;
      for (const element of elements) {
        if (!element.hasAttribute(attribute)) continue;
        if (attribute === 'href' && element.tagName.toLowerCase() === 'a') continue;
        kept.push(`${element.tagName.toLowerCase()}[${attribute}]`);
      }
      // `img[src]` is walked: the reference survives only as the core's to answer.
      if (attribute === 'src') {
        expect(result.imageRefs).toEqual(['https://probe.invalid/a.png']);
      }
    }
    // eslint-disable-next-line no-console -- the probe prints its own coverage
    console.log(
      `PROBE ${String(NETWORK_ATTRS.length)} network-capable attributes · ${String(cases)} cases · ${String(rendered)} rendered as elements · ${String(kept.length)} kept`,
    );
    // A probe that parsed nothing would pass vacuously: html must reach the sanitiser as nodes.
    expect(rendered).toBeGreaterThan(NETWORK_ATTRS.length * 50);
    expect(kept).toEqual([]);
  });
});

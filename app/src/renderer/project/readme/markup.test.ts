import { describe, expect, it } from 'vitest';

import type { ReadmeAsset } from '../../../generated/protocol';
import { applyAssets, renderMarkup, serialiseFragment } from './markup';

/**
 * §25.10's AC-P2-25-17 document, verbatim in shape: the five things a README can carry that a
 * panel must not act on.
 *
 * The **visible text** assertion is the load-bearing one. It proves `html: false` — a parser-level
 * guarantee that raw HTML is escaped rather than parsed — and not merely that a sanitiser ran
 * afterwards. Two layers, and this file can tell which one is doing the work because reverting
 * either turns a different assertion red (both proofs are recorded in the plan's report).
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
  return host.textContent ?? '';
}

function elementsOf(fragment: DocumentFragment): Element[] {
  const host = document.createElement('div');
  host.appendChild(fragment.cloneNode(true));
  return [...host.querySelectorAll('*')];
}

describe('the README pipeline', () => {
  it('readmeMarkup::ac_p2_25_17_hostile_markup_renders_as_visible_text', () => {
    const rendered = renderMarkup(HOSTILE);
    const text = textOf(rendered.fragment);
    const elements = elementsOf(rendered.fragment);

    // 1. The script tag is **text**, which is `html: false` and not the sanitiser.
    expect(text).toContain('<script>alert(1)</script>');
    expect(elements.some((el) => el.tagName.toLowerCase() === 'script')).toBe(false);

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
    for (const href of hrefs) {
      expect(href, href).toMatch(/^(?:https?:|mailto:|#|\/|[^a-z+.-]|[a-z+.-]+[^a-z+.-:])/iu);
      expect(href.toLowerCase().startsWith('javascript:')).toBe(false);
      expect(href.toLowerCase().startsWith('data:')).toBe(false);
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
    expect(rendered.imageRefs).toEqual(['https://cdn.example.test/badge.svg', 'docs/diagram.png']);
    expect(rendered.anchorCount).toBeGreaterThan(0);

    // 6. `<details>` came through as text too — `html: false` escapes every raw tag, not only
    //    the dangerous ones.
    expect(text).toContain('<details>');
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
    const rendered = renderMarkup('<script>alert(1)</script>\n');
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

import { describe, expect, it } from 'vitest';

import { HIGHLIGHT_LANGUAGES, highlightFence, KATEX_MAX_EXPAND, renderMath } from './highlight';
import { renderMarkup } from './markup';

function htmlOf(source: string): string {
  const host = document.createElement('div');
  host.appendChild(renderMarkup(source).fragment.cloneNode(true));
  return host.innerHTML;
}

function textOf(source: string): string {
  const host = document.createElement('div');
  host.appendChild(renderMarkup(source).fragment.cloneNode(true));
  return host.textContent;
}

describe('code fences', () => {
  it('colours a registered language', () => {
    const html = htmlOf('```rust\nfn main() {}\n```\n');
    expect(HIGHLIGHT_LANGUAGES).toContain('rust');
    expect(html).toContain('hljs-');
    expect(html).toContain('fn');
  });

  it('leaves an unregistered language escaped and uncoloured', () => {
    // Not in the subset, and never guessed at: `highlightAuto` would run every grammar over it.
    expect(HIGHLIGHT_LANGUAGES).not.toContain('brainfuck');
    const html = htmlOf('```brainfuck\n+[-->-[>>+>-----<<]<--<---]>-.\n```\n');
    expect(html).not.toContain('hljs-');
    expect(html).toContain('+[--&gt;-[&gt;&gt;+&gt;');
  });

  it('leaves a fence with no language alone', () => {
    const html = htmlOf('```\nplain <b>text</b>\n```\n');
    expect(html).not.toContain('hljs-');
    expect(html).toContain('&lt;b&gt;');
  });

  it('readmeMarkup::ac_p2_25_20_a_mermaid_fence_renders_its_source', () => {
    // §25.6: mermaid is JavaScript and the frame has no `allow-scripts`, so nothing would run
    // inside it — and running it in the parent would put untrusted-derived DOM in the document
    // that holds the bridge. The honest rendering is the diagram's own source.
    const source = '```mermaid\ngraph TD;\n  A-->B;\n```\n';
    expect(HIGHLIGHT_LANGUAGES).not.toContain('mermaid');
    const text = textOf(source);
    expect(text).toContain('graph TD;');
    expect(text).toContain('A-->B;');
    const html = htmlOf(source);
    expect(html).toContain('<pre>');
    expect(html).not.toContain('hljs-');
  });

  it('returns an empty string rather than guessing, which is what makes the source escape', () => {
    expect(highlightFence('fn main() {}', 'rust')).not.toBe('');
    expect(highlightFence('fn main() {}', 'mermaid')).toBe('');
    expect(highlightFence('fn main() {}', '')).toBe('');
    expect(highlightFence('fn main() {}', 'no-such-language')).toBe('');
  });
});

describe('math', () => {
  it('typesets an inline span as MathML and never as styled HTML', () => {
    const html = htmlOf('The identity $x^2 + y^2 = z^2$ holds.\n');
    expect(html).toContain('<math');
    // KaTeX's HTML output is what would need a font and a stylesheet; `output: 'mathml'` is what
    // keeps `font-src 'self'` untouched in a frame where `'self'` resolves to nothing.
    expect(html).not.toContain('katex-html');
  });

  it('renders no anchor for a \\href, which is trust: false', () => {
    const html = htmlOf('$\\href{https://example.test}{click}$\n');
    expect(html).not.toContain('<a ');
    expect(html.toLowerCase()).not.toContain('href="https://example.test"');
  });

  it('bounds macro expansion rather than looping on a definition bomb', () => {
    expect(KATEX_MAX_EXPAND).toBeGreaterThan(0);
    expect(KATEX_MAX_EXPAND).toBeLessThanOrEqual(10_000);
    const rendered = renderMath('\\def\\a{\\a}\\a', false);
    expect(typeof rendered).toBe('string');
  });

  it('leaves a lone dollar sign alone', () => {
    const text = textOf('It costs $5 and $6 in total.\n');
    expect(text).toContain('$5');
    expect(text).toContain('$6');
  });
});

/**
 * §25.5, stage 3: the frame that makes a sanitiser bypass worth nothing.
 *
 * The document that would otherwise hold the markup is the document holding the command bridge,
 * whose `KNOWN_COMMANDS` includes `projects.launch`. With `sandbox` present and **exactly empty**
 * the frame has an opaque origin and every flag off — no `allow-scripts`, no `allow-same-origin`,
 * no `allow-forms`, no `allow-popups`, no `allow-top-navigation`, no `allow-downloads`, no
 * storage, no pointer lock — and script execution is dead **twice**: the sandbox blocks it, and
 * `script-src 'self'` matches nothing from an opaque origin.
 *
 * Three rules about the `<style>` this builds:
 *
 * - **The type stack is a system stack**, because `font-src 'self'` resolves to nothing from an
 *   opaque origin: the frame cannot load the bundled brand faces however they are named.
 * - **No colour literal is typed into the frame.** §8.7's custom properties are read from the
 *   parent document at render time and injected, so a token change still has one owner. A value
 *   that does not match a conservative colour pattern is **dropped** rather than replaced by a
 *   literal, and the rule that used it falls back to `currentColor` or `transparent`.
 * - **Every `font-size` here is a member of §8.7's scale.** `scripts/check-type-scale.mjs` scans
 *   renderer `.ts` as well as `.css` and `.tsx`, so this template literal is in its walk.
 */
import { serialiseFragment } from './markup';

/**
 * The pipeline, re-exported so the panel reaches all of it through **one** dynamic `import()`.
 *
 * Four imports would produce four chunks and give `scripts/check-bundle.mjs` four names to chase;
 * one entry point is what makes "off the first-paint chunk" a single assertion.
 */
export { applyAssets, renderMarkup, serialiseFragment } from './markup';
export type { RenderedMarkup } from './markup';

/** The panel's two values, re-exported so this module is still the one door to the pipeline. */
export { FRAME_HEIGHT_PX, LINKS_INERT_NOTICE } from './frameFacts';

/** The §8.7 custom properties the frame's stylesheet uses, named once. */
export const FRAME_TOKEN_NAMES = [
  '--surface-2',
  '--surface-3',
  '--line-2',
  '--text-1',
  '--text-2',
  '--text-3',
] as const;

export type FrameTokenName = (typeof FRAME_TOKEN_NAMES)[number];

declare const validated: unique symbol;
/**
 * The resolved values, by property name — **and there is no way to build one that holds a value
 * this module did not validate**, because the brand is only applied by {@link frameTokens}.
 *
 * R100's shape: a state a type cannot construct needs no assertion guarding it. The alternative
 * was to validate at the point of interpolation and test that it happened, which is the same
 * guarantee with a test standing in for it.
 */
export type FrameTokens = Partial<Record<FrameTokenName, string>> & {
  readonly [validated]: true;
};

/**
 * A conservative colour pattern: a hex triple/quad or an `rgb()`/`rgba()` call, and nothing else.
 *
 * It is narrow on purpose. The values are read from the live document, so a stylesheet compromise
 * is the only way something else could arrive — and `url(…)` in a custom property would be a
 * request from inside a frame that is supposed to make none.
 */
const COLOUR = /^(?:#[0-9a-f]{3,8}|rgba?\([0-9.,\s%/]+\))$/iu;

/**
 * The only constructor. A value that is not a colour is **dropped**, never substituted: a literal
 * fallback typed here would be the second owner of a token this panel deliberately does not own.
 */
export function frameTokens(values: Partial<Record<FrameTokenName, string>>): FrameTokens {
  const kept: Partial<Record<FrameTokenName, string>> = {};
  for (const name of FRAME_TOKEN_NAMES) {
    const value = values[name]?.trim();
    if (value !== undefined && COLOUR.test(value)) kept[name] = value;
  }
  return kept as FrameTokens;
}

/** Read §8.7's tokens off a live element, through the one constructor. */
export function readFrameTokens(root: Element): FrameTokens {
  const resolved = getComputedStyle(root);
  const values: Partial<Record<FrameTokenName, string>> = {};
  for (const name of FRAME_TOKEN_NAMES) {
    values[name] = resolved.getPropertyValue(name).trim();
  }
  return frameTokens(values);
}

function tokenBlock(tokens: FrameTokens): string {
  const declared = FRAME_TOKEN_NAMES.filter((name) => tokens[name] !== undefined)
    .map((name) => `    ${name}: ${String(tokens[name])};`)
    .join('\n');
  return declared === '' ? '' : `  :root {\n${declared}\n  }\n`;
}

/**
 * The whole `srcdoc`: a doctype, a charset, one inline stylesheet and the serialised fragment.
 *
 * **Anchors render as text with no link affordance** — no underline, no accent colour, no pointer
 * cursor. A link that promises navigation and does nothing is the dead control §11.3a forbids,
 * and README anchors do not ride §25.2's opener (A9): they are pass-through URLs from hostile
 * content, and the opener's whole guarantee is that it never passes one through.
 */
export function buildSrcdoc(fragment: DocumentFragment, tokens: FrameTokens): string {
  const style = `
${tokenBlock(tokens)}  html, body {
    margin: 0;
    padding: 0;
    background: transparent;
    color: var(--text-2, currentColor);
  }
  body {
    padding: 15px 16px 17px;
    font-family: ui-sans-serif, system-ui, sans-serif;
    font-size: 12.5px;
    line-height: 1.6;
    overflow-wrap: anywhere;
  }
  h1, h2, h3, h4, h5, h6 { color: var(--text-1, currentColor); line-height: 1.3; }
  h1 { font-size: 21px; }
  h2 { font-size: 17px; }
  h3, h4, h5, h6 { font-size: 14px; }
  a {
    color: inherit;
    text-decoration: none;
    cursor: default;
  }
  code, pre, kbd, samp {
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 11px;
  }
  pre {
    padding: 10px 12px;
    overflow-x: auto;
    background: var(--surface-2, transparent);
    border: 1px solid var(--line-2, currentColor);
  }
  blockquote {
    margin: 12px 0;
    padding-left: 12px;
    border-left: 2px solid var(--line-2, currentColor);
    color: var(--text-3, currentColor);
  }
  table { border-collapse: collapse; }
  th, td {
    padding: 4px 8px;
    border: 1px solid var(--line-2, currentColor);
    text-align: left;
  }
  th { background: var(--surface-3, transparent); }
  img { max-width: 100%; height: auto; }
  .cdt-readme-asset {
    display: inline-block;
    padding: 2px 6px;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 7.5px;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--text-3, currentColor);
    border: 1px dashed var(--line-2, currentColor);
  }
  .hljs-comment, .hljs-quote { color: var(--text-3, currentColor); }
  .hljs-keyword, .hljs-selector-tag, .hljs-literal, .hljs-title, .hljs-name {
    color: var(--text-1, currentColor);
  }
`;
  return [
    '<!doctype html>',
    '<meta charset="utf-8">',
    `<style>${style}</style>`,
    serialiseFragment(fragment),
  ].join('');
}

/**
 * §25.5, stages 1, 2 and 2½: parse, sanitise, and take every image out of the document.
 *
 * **Two layers, and the first is a parser-level guarantee.** `markdown-it` with `html: false`
 * escapes raw HTML in the source to text rather than parsing it, so a `<script>` in a README is
 * visible characters and never a node. DOMPurify is the second layer and is still needed, because
 * markdown carries URLs of its own — `[x](javascript:…)` — and because the highlighter's and
 * KaTeX's output must pass the same gate as everything else.
 *
 * **The allowlist is written out here, not inherited.** A default profile changes with the
 * dependency; this list changes only when somebody edits this file.
 *
 * **DOMPurify is never reconfigured to admit `data:` URIs.** Widening its URI regexp is how
 * `data:text/html` in an `href` becomes XSS. The substitution in {@link applyAssets} happens
 * *after* the sanitiser, on nodes the sanitiser produced, through the DOM API — and only after
 * the value has been checked against {@link ALLOWED_ASSET_MEDIA_TYPES}.
 */
import DOMPurify from 'dompurify';
import MarkdownIt from 'markdown-it';
import type { StateInline } from 'markdown-it';

import type { ReadmeAsset } from '../../../generated/protocol';
import { highlightFence, renderMath } from './highlight';

/** The class a blocked or refused image renders as, and what the frame's stylesheet targets. */
export const ASSET_PLACEHOLDER_CLASS = 'cdt-readme-asset';
/** The attribute carrying the reference a placeholder stands for. */
export const ASSET_REF_ATTR = 'data-readme-ref';
/** The attribute carrying §25.5's five states, so the panel and the census can read one. */
export const ASSET_STATE_ATTR = 'data-asset-state';

/**
 * What a placeholder carries **before anything has been asked about it**.
 *
 * Not `blocked`: that is the consent state, and a reference nobody has asked the core about yet is
 * not one consent refused. The distinction is visible in the built app — the end-to-end census
 * reads this attribute on the first frame and after the round trip, and with `blocked` as the
 * initial value the two frames were indistinguishable.
 */
export const ASSET_STATE_UNASKED = 'unasked';

/**
 * The five media types the core is allowed to hand back, restated here because this is the side
 * that decides whether a `data:` URI reaches a document. The core's list is
 * `core/src/readme/assets.rs`'s `ALLOWED_MEDIA_TYPES`, and `app/test/acceptance/readmePanel.test.ts`
 * reads the Rust constant to prove the two agree — a cross-language mirror needs a test that
 * reads the other side.
 */
export const ALLOWED_ASSET_MEDIA_TYPES: readonly string[] = [
  'image/png',
  'image/jpeg',
  'image/gif',
  'image/webp',
  'image/svg+xml',
];

/**
 * What a README may contain after sanitising.
 *
 * The MathML names are KaTeX's output (§25.5's `output: 'mathml'`): MathML needs no font file and
 * no stylesheet, which is what keeps `font-src 'self'` untouched in a frame where `'self'`
 * resolves to nothing.
 */
export const ALLOWED_TAGS: readonly string[] = [
  // Prose
  'p',
  'br',
  'hr',
  'blockquote',
  'pre',
  'code',
  'span',
  'div',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'strong',
  'em',
  'b',
  'i',
  'u',
  's',
  'del',
  'ins',
  'mark',
  'sub',
  'sup',
  'small',
  'kbd',
  'samp',
  'abbr',
  'cite',
  'q',
  'dfn',
  'time',
  'var',
  // Lists
  'ul',
  'ol',
  'li',
  'dl',
  'dt',
  'dd',
  // Tables (GFM)
  'table',
  'thead',
  'tbody',
  'tfoot',
  'tr',
  'th',
  'td',
  'caption',
  'colgroup',
  'col',
  // Links and images. `img` is admitted so the walk below can *find* one and take its `src`
  // away; nothing that reaches the frame keeps the attribute.
  'a',
  'img',
  // Disclosure, which a README uses for collapsed sections
  'details',
  'summary',
  // MathML, KaTeX's `output: 'mathml'`
  'math',
  'semantics',
  'annotation',
  'mrow',
  'mi',
  'mo',
  'mn',
  'ms',
  'mtext',
  'mspace',
  'msup',
  'msub',
  'msubsup',
  'mfrac',
  'msqrt',
  'mroot',
  'mstyle',
  'munder',
  'mover',
  'munderover',
  'mmultiscripts',
  'mtable',
  'mtr',
  'mtd',
  'mpadded',
  'mphantom',
  'menclose',
  'merror',
  'maction',
];

/** Every attribute a sanitised document may keep. No `style`, no `on*`, no `data-*`. */
export const ALLOWED_ATTR: readonly string[] = [
  'href',
  'title',
  'alt',
  'src',
  'class',
  'lang',
  'dir',
  'colspan',
  'rowspan',
  'align',
  'start',
  'reversed',
  'open',
  'datetime',
  'cite',
  // MathML
  'display',
  'xmlns',
  'encoding',
  'mathvariant',
  'displaystyle',
  'scriptlevel',
  'stretchy',
  'fence',
  'separator',
  'accent',
  'accentunder',
  'linethickness',
  'columnalign',
  'rowspacing',
  'columnspacing',
  'lspace',
  'rspace',
  'width',
  'height',
  'depth',
  'voffset',
  'notation',
];

/** Stated even though the allowlist already excludes them: a second, explicit refusal. */
export const FORBID_TAGS: readonly string[] = [
  'script',
  'style',
  'iframe',
  'object',
  'embed',
  'form',
  'input',
  'base',
  'link',
  'meta',
  'svg',
];

/** What one README parse produced. */
export interface RenderedMarkup {
  /** The sanitised document, with every image already replaced by a placeholder. */
  readonly fragment: DocumentFragment;
  /** Every `img[src]` value, in document order, de-duplicated nowhere: the core does that. */
  readonly imageRefs: readonly string[];
  /** How many anchors survived, which is what decides whether the panel states they are inert. */
  readonly anchorCount: number;
}

/**
 * The parser, built once.
 *
 * `linkify: true` turns a bare URL into an anchor, which is what a reader expects and is inert
 * inside the frame like every other anchor. `typographer: false` because a README is source text
 * and replacing its quotes is an edit.
 */
const parser = new MarkdownIt({
  html: false,
  linkify: true,
  typographer: false,
  // §25.5: the highlighter plugs in here rather than in a second AST pass, and its output goes
  // through the sanitiser like everything else — the allowlist admits `<span class>` and nothing
  // more. An unregistered language returns `''`, which is what makes markdown-it escape the
  // source itself.
  highlight: (code: string, language: string) => highlightFence(code, language),
});

/**
 * `$…$` and `$$…$$`, as an inline rule.
 *
 * It is a rule rather than a pass over the rendered text because KaTeX's output must reach the
 * **sanitiser**: a substitution performed afterwards would put unsanitised markup into a
 * sanitised document, which is the one ordering this whole pipeline exists to get right.
 *
 * Deliberately narrow, and the limits are stated rather than discovered: no multi-line `$$` block
 * — a display span is one line — and a delimiter with whitespace just inside it is not a
 * delimiter, which is what keeps `it costs $5 and $6` out of the typesetter.
 */
function mathInline(state: StateInline, silent: boolean): boolean {
  const start = state.pos;
  if (state.src.charAt(start) !== '$') return false;
  const display = state.src.startsWith('$$', start);
  const marker = display ? '$$' : '$';
  const from = start + marker.length;
  if (/^\s|^$/u.test(state.src.charAt(from))) return false;

  let pos = from;
  let close = -1;
  while (pos < state.posMax) {
    if (state.src.charAt(pos) === '\\') {
      pos += 2;
      continue;
    }
    if (state.src.startsWith(marker, pos)) {
      close = pos;
      break;
    }
    pos += 1;
  }
  if (close < 0 || close === from || /\s/u.test(state.src.charAt(close - 1))) return false;

  if (!silent) {
    const token = state.push(display ? 'math_display' : 'math_inline', 'math', 0);
    token.content = state.src.slice(from, close);
    token.markup = marker;
  }
  state.pos = close + marker.length;
  return true;
}

parser.inline.ruler.before('escape', 'math_inline', mathInline);
parser.renderer.rules['math_inline'] = (tokens, index) =>
  renderMath(tokens[index]?.content ?? '', false);
parser.renderer.rules['math_display'] = (tokens, index) =>
  renderMath(tokens[index]?.content ?? '', true);

/** The parser, exposed so §25.6's highlighter can install its hook without a second instance. */
export function markdownParser(): MarkdownIt {
  return parser;
}

function sanitiseConfig(): Parameters<typeof DOMPurify.sanitize>[1] {
  return {
    ALLOWED_TAGS: [...ALLOWED_TAGS],
    ALLOWED_ATTR: [...ALLOWED_ATTR],
    FORBID_TAGS: [...FORBID_TAGS],
    ALLOW_DATA_ATTR: false,
    ALLOW_ARIA_ATTR: false,
    ALLOW_UNKNOWN_PROTOCOLS: false,
    RETURN_DOM_FRAGMENT: true,
  };
}

/**
 * Parse, sanitise, and take every image out of the document.
 *
 * **The image walk is why the panel's first frame issues zero requests of any kind.** Both the
 * remote and the local case start blocked: the `src` is removed from a node the sanitiser
 * produced and the value is handed to the core, which decides whether any bytes come back.
 */
export function renderMarkup(source: string): RenderedMarkup {
  const html = parser.render(source);
  const fragment = DOMPurify.sanitize(html, sanitiseConfig()) as unknown as DocumentFragment;

  const imageRefs: string[] = [];
  for (const image of [...fragment.querySelectorAll('img')]) {
    const reference = image.getAttribute('src');
    const alt = image.getAttribute('alt') ?? '';
    const placeholder = image.ownerDocument.createElement('span');
    placeholder.className = ASSET_PLACEHOLDER_CLASS;
    placeholder.setAttribute(ASSET_STATE_ATTR, ASSET_STATE_UNASKED);
    placeholder.textContent = alt;
    if (reference !== null && reference !== '') {
      placeholder.setAttribute(ASSET_REF_ATTR, reference);
      imageRefs.push(reference);
    }
    image.replaceWith(placeholder);
  }

  return {
    fragment,
    imageRefs,
    anchorCount: fragment.querySelectorAll('a').length,
  };
}

/** Whether a value is a `data:` URI naming one of the five media types, and nothing else. */
function isAllowedAssetUri(value: string | null): value is string {
  if (value === null || !value.startsWith('data:')) return false;
  const media = value.slice('data:'.length).split(';')[0]?.split(',')[0] ?? '';
  return ALLOWED_ASSET_MEDIA_TYPES.includes(media.toLowerCase());
}

/**
 * Put the core's bytes back on the nodes their references came from.
 *
 * A row whose state is not `ok`, or whose `dataUri` is not a `data:` URI naming one of the five
 * media types, leaves the placeholder standing with its state recorded on it. **`blocked` is the
 * consent state and is never rendered as a failure** — the panel reads the attribute and says so
 * in its own voice, below the frame.
 */
export function applyAssets(fragment: DocumentFragment, assets: readonly ReadmeAsset[]): void {
  const byRef = new Map(assets.map((asset) => [asset.ref, asset]));
  for (const placeholder of [...fragment.querySelectorAll(`[${ASSET_REF_ATTR}]`)]) {
    const reference = placeholder.getAttribute(ASSET_REF_ATTR);
    const asset = reference === null ? undefined : byRef.get(reference);
    if (asset === undefined) continue;
    if (asset.state !== 'ok' || !isAllowedAssetUri(asset.dataUri)) {
      placeholder.setAttribute(ASSET_STATE_ATTR, asset.state);
      continue;
    }
    const image = placeholder.ownerDocument.createElement('img');
    image.setAttribute('src', asset.dataUri);
    image.setAttribute('alt', placeholder.textContent ?? '');
    image.className = 'cdt-readme-image';
    placeholder.replaceWith(image);
  }
}

/**
 * The fragment as HTML text, escaped by the DOM rather than by a template.
 *
 * `innerHTML` on a detached host is what escapes both text nodes and attribute values; building
 * the string by hand is how a sanitised document becomes an unsanitised one on the way out.
 */
export function serialiseFragment(fragment: DocumentFragment): string {
  const host = fragment.ownerDocument.createElement('div');
  host.appendChild(fragment.cloneNode(true));
  return host.innerHTML;
}

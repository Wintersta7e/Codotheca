/**
 * §25.5's two decorations: a code fence's colour and a math span's typesetting.
 *
 * **`highlightAuto` is forbidden.** It runs every registered grammar over the input, so a
 * pathological fence is a stall on the thread that paints the shelf. The subset below is
 * registered explicitly and is the only place it is named; an **unregistered** language renders
 * as a plain `<pre><code>` with the source escaped — no colour, no fabrication — and so does a
 * fence with no language at all.
 *
 * **Mermaid does not ship** (§25.6). It is not in the subset, so a ` ```mermaid ` fence takes the
 * unregistered path and renders its own source, which is honest: it shows the diagram source
 * rather than a broken frame or a fabricated picture. Nothing here special-cases it, and
 * `highlight.test.ts` asserts the outcome rather than the absence of a branch.
 *
 * **KaTeX renders to MathML.** That is what keeps `font-src 'self'` untouched: MathML needs no
 * font file and no stylesheet, and `'self'` resolves to nothing from the frame's opaque origin.
 * `trust: false` disables `\href`, `\url` and `\includegraphics`; the bounded `maxExpand` is the
 * guard against a `\def` bomb. The trade-off is real and stated: KaTeX's HTML output typesets
 * better, and it is not worth a CSP row plus several hundred KB of base64 fonts in every srcdoc.
 */
import hljs from 'highlight.js/lib/core';
import bash from 'highlight.js/lib/languages/bash';
import c from 'highlight.js/lib/languages/c';
import cpp from 'highlight.js/lib/languages/cpp';
import csharp from 'highlight.js/lib/languages/csharp';
import css from 'highlight.js/lib/languages/css';
import diff from 'highlight.js/lib/languages/diff';
import go from 'highlight.js/lib/languages/go';
import ini from 'highlight.js/lib/languages/ini';
import java from 'highlight.js/lib/languages/java';
import javascript from 'highlight.js/lib/languages/javascript';
import json from 'highlight.js/lib/languages/json';
import markdown from 'highlight.js/lib/languages/markdown';
import python from 'highlight.js/lib/languages/python';
import ruby from 'highlight.js/lib/languages/ruby';
import rust from 'highlight.js/lib/languages/rust';
import shell from 'highlight.js/lib/languages/shell';
import sql from 'highlight.js/lib/languages/sql';
import typescript from 'highlight.js/lib/languages/typescript';
import xml from 'highlight.js/lib/languages/xml';
import yaml from 'highlight.js/lib/languages/yaml';
import katex from 'katex';

/** KaTeX's macro-expansion budget. A `\def` bomb is an infinite loop on the paint thread. */
export const KATEX_MAX_EXPAND = 1000;

const GRAMMARS: ReadonlyArray<readonly [string, Parameters<typeof hljs.registerLanguage>[1]]> = [
  ['bash', bash],
  ['c', c],
  ['cpp', cpp],
  ['csharp', csharp],
  ['css', css],
  ['diff', diff],
  ['go', go],
  ['ini', ini],
  ['java', java],
  ['javascript', javascript],
  ['json', json],
  ['markdown', markdown],
  ['python', python],
  ['ruby', ruby],
  ['rust', rust],
  ['shell', shell],
  ['sql', sql],
  ['typescript', typescript],
  ['xml', xml],
  ['yaml', yaml],
];

/** The registered subset, named once. A language outside it renders uncoloured, never guessed. */
export const HIGHLIGHT_LANGUAGES: readonly string[] = GRAMMARS.map(([name]) => name);

let registered = false;
function registerOnce(): void {
  if (registered) return;
  for (const [name, grammar] of GRAMMARS) hljs.registerLanguage(name, grammar);
  registered = true;
}

/**
 * The highlighted inner HTML of one fence, or `''` when this build has no grammar for it.
 *
 * `''` is what tells `markdown-it` to escape the source itself, which is the honest rendering for
 * an unregistered language: the reader sees exactly what the document says.
 */
export function highlightFence(code: string, language: string): string {
  registerOnce();
  const name = language.trim().toLowerCase();
  if (name === '' || !HIGHLIGHT_LANGUAGES.includes(name)) return '';
  try {
    // `ignoreIllegals` keeps a fence that is *nearly* the language it claims from throwing; the
    // alternative is a code block that vanishes because its sample had a typo.
    return hljs.highlight(code, { language: name, ignoreIllegals: true }).value;
  } catch {
    return '';
  }
}

/**
 * One math span as MathML, or the source escaped when KaTeX cannot read it.
 *
 * `throwOnError: false` renders KaTeX's own error markup rather than throwing, and that markup
 * passes the same sanitiser as everything else — its inline `style` is not on the allowlist and
 * is dropped.
 */
export function renderMath(tex: string, displayMode: boolean): string {
  return katex.renderToString(tex, {
    output: 'mathml',
    trust: false,
    throwOnError: false,
    strict: false,
    maxExpand: KATEX_MAX_EXPAND,
    displayMode,
  });
}

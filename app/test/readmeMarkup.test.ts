import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * Criterion 42's **Peek half**, and only that half.
 *
 * §8.4 ruled the README renders "as plain text, never rendered markup" on both surfaces, and this
 * file used to grep `ReadmePanel.tsx` for the five ways a parser gets in. [p2] §25.5 buys the
 * project page a parser and a sandbox, so that half moved: the page's bar is §25.10's
 * AC-P2-25-16 … 22, which assert the frame, its sandbox and the hostile document rather than the
 * absence of a renderer.
 *
 * **Peek's half is kept word for word.** Mounting a document per expanded card on the surface
 * measured at 1,000 virtualized cards is a different decision with different evidence, and §8.4's
 * whole point is triage speed — so `peekText.ts` stays plain text and this is what says so.
 *
 * It lives in the node project because the renderer project carries no Node types by design
 * (`tsconfig.web.json` gives it `types: ["vite/client"]` and nothing else).
 */
const peek = fileURLToPath(new URL('../src/renderer/shelf/peekText.ts', import.meta.url));
const source = readFileSync(peek, 'utf8');

describe('Peek has no way to render markup at all', () => {
  it('read real source, or every assertion below is vacuous', () => {
    expect(source.length).toBeGreaterThan(500);
    expect(source).toContain('firstParagraph');
  });

  it('carries none of the five ways a parser gets in', () => {
    for (const banned of ['dangerouslySetInnerHTML', 'innerHTML', '<a ', '<img', 'marked']) {
      expect(source).not.toContain(banned);
    }
  });

  it('and none of §25.5 reaches it either — the frame is the project page and nowhere else', () => {
    for (const banned of ['markdown-it', 'dompurify', 'sandbox', 'srcdoc', 'srcDoc']) {
      expect(source).not.toContain(banned);
    }
  });
});

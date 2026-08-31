import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * Criterion 42's other half. §8.4 ruled the README renders "as plain text, never rendered
 * markup" and §8.5.3 does not relax it by giving the panel more room: the page adds width, not a
 * parser. The rendered assertion lives beside the component; this one greps the source, so a
 * later "just render the links" cannot pass review by accident.
 *
 * It lives in the node project because the renderer project carries no Node types by design
 * (`tsconfig.web.json` gives it `types: ["vite/client"]` and nothing else).
 */
const panel = fileURLToPath(
  new URL('../src/renderer/project/readme/ReadmePanel.tsx', import.meta.url),
);
const source = readFileSync(panel, 'utf8');

describe('the README panel has no way to render markup at all', () => {
  it('read real source, or every assertion below is vacuous', () => {
    expect(source.length).toBeGreaterThan(500);
    expect(source).toContain('ReadmePanel');
  });

  it('carries none of the five ways a parser gets in', () => {
    for (const banned of ['dangerouslySetInnerHTML', 'innerHTML', '<a ', '<img', 'marked']) {
      expect(source).not.toContain(banned);
    }
  });
});

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import { SIDECAR_COUNT_NOUNS } from '../src/renderer/failure/copy';

/**
 * §48.7.2's window lists a sidecar's counts under their nouns, keyed as the core keys them: every
 * base count, then one per registered section. A key with no noun would be a figure the window
 * cannot name, so each is read from the core's own declarations rather than restated here.
 */
describe('the sidecar count nouns', () => {
  const RUST = readFileSync(
    fileURLToPath(new URL('../../core/src/index/sidecar.rs', import.meta.url)),
    'utf8',
  );

  const block = (open: string): string => {
    const at = RUST.indexOf(open);
    expect(at, `the core no longer declares \`${open}\``).toBeGreaterThanOrEqual(0);
    return RUST.slice(at, RUST.indexOf('];', at));
  };

  it('names every count key the core writes and every registered section', () => {
    const base = [...block('pub const BASE_COUNT_KEYS').matchAll(/"([a-z_]+)"/gu)].map(
      (m) => m[1] ?? '',
    );
    const sections = [...block('pub const SECTIONS').matchAll(/\bname: "([a-z_]+)"/gu)].map(
      (m) => m[1] ?? '',
    );
    // eslint-disable-next-line no-console -- the count compared is the evidence
    console.log(
      `sidecar nouns: ${String(base.length)} base count keys, ` +
        `${String(sections.length)} sections`,
    );
    expect(base.length).toBeGreaterThan(0);
    expect(sections.length).toBeGreaterThan(0);

    const unnamed = [...base, ...sections].filter(
      (key) => !Object.hasOwn(SIDECAR_COUNT_NOUNS, key),
    );
    expect(unnamed).toEqual([]);
  });
});

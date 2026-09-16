import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

/**
 * Acceptance: [p2] §25.5's **cross-language values**.
 *
 * One value stated twice drifts, and this section states three across the process boundary: the
 * five media types the core will hand back and the renderer will accept, the per-asset cap, and
 * the reply cap that keeps an answer inside the frame. **A cross-language mirror needs a test that
 * reads the other side** — so this reads the Rust, rather than a second copy of the numbers.
 *
 * The panel's rendered bar is `test/dom/readmePanel.test.ts`; the pipeline's is beside the module.
 */
const repoRoot = fileURLToPath(new URL('../../..', import.meta.url));
const assetsRs = readFileSync(`${repoRoot}/core/src/readme/assets.rs`, 'utf8');
const markupTs = readFileSync(`${repoRoot}/app/src/renderer/project/readme/markup.ts`, 'utf8');
const frameTs = readFileSync(`${repoRoot}/app/src/main/core/frame.ts`, 'utf8');

/** The `&str` literals of a Rust array, in order. */
function rustArray(source: string, name: string): string[] {
  const block = new RegExp(`${name}[^=]*=\\s*\\[([^\\]]*)\\]`, 'u').exec(source);
  if (block === null) throw new Error(`core/src/readme/assets.rs declares no ${name}`);
  return [...(block[1] ?? '').matchAll(/"([^"]+)"/gu)].map((match) => match[1] ?? '');
}

/** A `const NAME: usize = <expr>;` evaluated as arithmetic over integers. */
function rustConst(source: string, name: string): number {
  const found = new RegExp(`const ${name}: usize = ([^;]+);`, 'u').exec(source);
  if (found === null) throw new Error(`core/src/readme/assets.rs declares no ${name}`);
  const expression = (found[1] ?? '').replace(/_/gu, '');
  if (!/^[0-9*+\s]+$/u.test(expression)) throw new Error(`${name} is not arithmetic`);
  return expression
    .split('+')
    .map((term) => term.split('*').reduce((product, n) => product * Number(n.trim()), 1))
    .reduce((sum, n) => sum + n, 0);
}

describe('§25.5 the media types have one meaning on both sides', () => {
  it('reads real source, or every assertion below is vacuous', () => {
    expect(assetsRs.length).toBeGreaterThan(2000);
    expect(markupTs.length).toBeGreaterThan(2000);
  });

  it('admits the same five, in the same order', () => {
    const core = rustArray(assetsRs, 'ALLOWED_MEDIA_TYPES');
    const renderer = [
      ...(/ALLOWED_ASSET_MEDIA_TYPES[^=]*=\s*\[([^\]]*)\]/u.exec(markupTs)?.[1] ?? '').matchAll(
        /'([^']+)'/gu,
      ),
    ].map((match) => match[1] ?? '');
    expect(core).toEqual(['image/png', 'image/jpeg', 'image/gif', 'image/webp', 'image/svg+xml']);
    expect(renderer).toEqual(core);
  });

  it('keeps the reply inside the frame cap by arithmetic, not by hope', () => {
    const perAsset = rustConst(assetsRs, 'ASSET_BYTE_CAP');
    const perReply = rustConst(assetsRs, 'REPLY_BYTE_CAP');
    const maxFrame = Number(
      /MAX_FRAME_BYTES\s*=\s*([0-9*_\s]+);/u
        .exec(frameTs)?.[1]
        ?.split('*')
        .reduce((product, n) => product * Number(n.replace(/_/gu, '').trim()), 1),
    );

    expect(perAsset).toBe(512 * 1024);
    expect(perReply).toBe(4 * 1024 * 1024);
    expect(maxFrame).toBe(8 * 1024 * 1024);
    // 4 MB of source bytes is ~5.33 MB of base64, which is what the reply carries. The cap is on
    // the **source** bytes for that reason: capping the encoded size would cap the wrong number.
    expect(Math.ceil((perReply / 3) * 4)).toBeLessThan(maxFrame);
    expect(perAsset).toBeLessThan(perReply);
  });
});

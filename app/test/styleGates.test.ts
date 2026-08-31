import { spawnSync } from 'node:child_process';
import { readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const repo = fileURLToPath(new URL('../../', import.meta.url));

/**
 * `spawnSync`, not `execFileSync`: these gates report on **stderr**, as every script in
 * `scripts/` does, so that stdout stays free for anything a caller wants to pipe. `execFileSync`
 * hands back stdout alone on success, which is empty here and makes a passing gate look silent.
 */
function run(script: string): { code: number; out: string } {
  const result = spawnSync('node', [script], {
    cwd: repo,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  return { code: result.status ?? 1, out: `${result.stdout}${result.stderr}` };
}

/**
 * Every gate here reports what it scanned, and every test asserts that number is not zero.
 * A gate whose passing run reads no files is a failing gate that looks green — this repository
 * has already shipped two, a CI grep over gitignored paths and a motion clamp asserting an
 * attribute the product does not set.
 */
function scanned(out: string, label: string): number {
  const m = new RegExp(`(\\d+) ${label}`).exec(out);
  expect(m, `${label} count missing from: ${out}`).not.toBeNull();
  return Number(m?.[1]);
}

const STYLES = fileURLToPath(new URL('../src/renderer/styles/', import.meta.url));

/** Drop a stylesheet into the directory the gates scan, run one, and take the file away again. */
function gateOver(script: string, css: string): { code: number; out: string } {
  const probe = join(STYLES, '__probe.css');
  writeFileSync(probe, css, 'utf8');
  try {
    return run(script);
  } finally {
    rmSync(probe, { force: true });
  }
}

describe('the stylesheet gate', () => {
  it('passes over the renderer stylesheet as written, having read it', () => {
    const result = run('scripts/check-style-tokens.mjs');
    expect(result.out).toContain('check-style-tokens: ok');
    expect(result.code).toBe(0);
    expect(scanned(result.out, 'css file')).toBeGreaterThan(0);
  });

  it('rejects an off-token colour, an unlisted duration and an unlisted curve', () => {
    const result = gateOver(
      'scripts/check-style-tokens.mjs',
      '.cdt-probe { color: #7a8896; transition: opacity .25s ease-in; }\n',
    );
    expect(result.code).toBe(1);
    expect(result.out).toContain('colour literal outside the token block: #7a8896');
    expect(result.out).toContain('duration .25s is not in §11.6');
    expect(result.out).toContain('timing function not in §11.6: ease-in');
  });

  it('accepts the leading-dot form of a duration §11.6 does carry', () => {
    // `.3s` is 300ms — the brackets' travel. Reading the dot as a separator turns it into 3s
    // and rejects a legal value, which is the quieter half of the same defect as the line above.
    const result = gateOver(
      'scripts/check-style-tokens.mjs',
      '.cdt-probe { transition: opacity .3s cubic-bezier(.2,.85,.2,1); }\n',
    );
    expect(result.out).not.toContain('is not in §11.6');
    expect(result.code).toBe(0);
  });

  it('reads a curve as four numbers, not as a spelling', () => {
    // The spec writes `cubic-bezier(.2,.85,.2,1)`; a CSS formatter writes
    // `cubic-bezier(0.2, 0.85, 0.2, 1)`. Both are §11.6's standard enter curve. Comparing raw
    // strings fails on a leading zero, which says nothing about the motion contract.
    const listed = gateOver(
      'scripts/check-style-tokens.mjs',
      '.cdt-probe { transition: opacity 160ms cubic-bezier(0.2, 0.85, 0.2, 1); }\n',
    );
    expect(listed.out).toContain('check-style-tokens: ok');
    expect(listed.code).toBe(0);

    // …and the normalisation must not have made it permissive.
    const unlisted = gateOver(
      'scripts/check-style-tokens.mjs',
      '.cdt-probe { transition: opacity 160ms cubic-bezier(0.2, 1.5, 0.4, 1); }\n',
    );
    expect(unlisted.code).toBe(1);
    expect(unlisted.out).toContain('timing function not in §11.6');
  });

  it('rejects a var() that names no token', () => {
    const result = gateOver(
      'scripts/check-style-tokens.mjs',
      '.cdt-probe { color: var(--text-9); }\n',
    );
    expect(result.code).toBe(1);
    expect(result.out).toContain('var(--text-9) is not declared in tokens.css');
  });

  it('rejects a second declaration of a shared entry keyframe', () => {
    // R35(b), and the half nothing else can catch: a duplicate `@keyframes` does not error, the
    // later body silently wins, and a screen-local copy is invisible to motion.css's tier clamp.
    const result = gateOver(
      'scripts/check-style-tokens.mjs',
      '@keyframes viewIn { from { opacity: 0 } to { opacity: 1 } }\n',
    );
    expect(result.code).toBe(1);
    expect(result.out).toContain('@keyframes viewIn is declared 2 times');
    expect(result.out).toContain('belongs in styles/base.css');
  });
});

describe('the type gate', () => {
  const typeGate = 'scripts/check-type-scale.mjs';

  it('passes over the renderer as written, having read it', () => {
    const result = run(typeGate);
    expect(result.out).toContain('check-type-scale: ok');
    expect(result.code).toBe(0);
    expect(scanned(result.out, 'file')).toBeGreaterThan(0);
    expect(scanned(result.out, 'scale member')).toBeGreaterThan(0);
  });

  it('rejects the 7px floor breach, an off-scale size and a size with no tracking', () => {
    const result = gateOver(
      typeGate,
      '.cdt-probe { font-family: var(--font-mono); font-size: 6.5px; }\n',
    );
    expect(result.code).toBe(1);
    expect(result.out).toContain('6.5px is below the 7px floor');
    expect(result.out).toContain('6.5px appears, and it renders nowhere');
    expect(result.out).toContain('6.5px ships without its tracking');
  });

  it('demands tracking across the lines a real stylesheet is written on', () => {
    // The size and its family sit on separate lines in every stylesheet anyone writes. A block
    // splitter that also breaks on `;` puts them in different blocks, and the whole tracking
    // clause then fires on nothing — green against a smear.
    const result = gateOver(
      typeGate,
      '.cdt-probe {\n  font-family: var(--font-mono);\n  font-size: 7.5px;\n}\n',
    );
    expect(result.code).toBe(1);
    expect(result.out).toContain('7.5px ships without its tracking');
  });

  it('accepts a tracked mono size that is on the scale', () => {
    const result = gateOver(
      typeGate,
      '.cdt-probe {\n  font-family: var(--font-mono);\n  font-size: 7.5px;\n  letter-spacing: .14em;\n}\n',
    );
    expect(result.out).toContain('check-type-scale: ok');
    expect(result.code).toBe(0);
  });
});

describe('the focus-ring gate', () => {
  const focusGate = 'scripts/check-focus-ring.mjs';

  it('passes over the renderer as written, having read it', () => {
    const result = run(focusGate);
    expect(result.out).toContain('check-focus-ring: ok');
    expect(result.code).toBe(0);
    expect(scanned(result.out, 'file')).toBeGreaterThan(0);
  });

  it('rejects an outline removed with no replacement in the same rule', () => {
    const result = gateOver(focusGate, '.cdt-probe:focus-visible { outline: none; }\n');
    expect(result.code).toBe(1);
    expect(result.out).toContain('outline removed with no inset box-shadow ring in the same rule');
  });

  it('accepts an outline replaced by an inset ring in the same rule', () => {
    // The ring has to be inset and on the element itself: `clip-path` deletes an outer shadow
    // exactly as it deletes an outline, and an overlay layer once failed to mount unnoticed.
    const result = gateOver(
      focusGate,
      '.cdt-probe:focus-visible {\n  outline: none;\n  box-shadow: inset 0 0 0 2px var(--sig);\n}\n',
    );
    expect(result.out).toContain('check-focus-ring: ok');
    expect(result.code).toBe(0);
  });
});

describe('the three gates are wired into npm run lint', () => {
  it('names all of them, or a green lint proves nothing about the stylesheet', () => {
    const pkg: unknown = JSON.parse(readFileSync(join(repo, 'package.json'), 'utf8'));
    const scripts = (pkg as { scripts?: Record<string, string> }).scripts ?? {};
    expect(scripts['lint']).toContain('lint:style');
    expect(scripts['lint']).toContain('lint:type');
    expect(scripts['lint']).toContain('lint:focus');
  });
});

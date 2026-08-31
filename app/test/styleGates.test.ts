import { spawnSync } from 'node:child_process';
import { rmSync, writeFileSync } from 'node:fs';
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

/** Drop a stylesheet into the directory the gate scans, run it, and take the file away again. */
function gateOver(css: string): { code: number; out: string } {
  const probe = join(STYLES, '__probe.css');
  writeFileSync(probe, css, 'utf8');
  try {
    return run('scripts/check-style-tokens.mjs');
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
    const result = gateOver('.cdt-probe { color: #7a8896; transition: opacity .25s ease-in; }\n');
    expect(result.code).toBe(1);
    expect(result.out).toContain('colour literal outside the token block: #7a8896');
    expect(result.out).toContain('duration .25s is not in §11.6');
    expect(result.out).toContain('timing function not in §11.6: ease-in');
  });

  it('accepts the leading-dot form of a duration §11.6 does carry', () => {
    // `.3s` is 300ms — the brackets' travel. Reading the dot as a separator turns it into 3s
    // and rejects a legal value, which is the quieter half of the same defect as the line above.
    const result = gateOver('.cdt-probe { transition: opacity .3s cubic-bezier(.2,.85,.2,1); }\n');
    expect(result.out).not.toContain('is not in §11.6');
    expect(result.code).toBe(0);
  });

  it('rejects a var() that names no token', () => {
    const result = gateOver('.cdt-probe { color: var(--text-9); }\n');
    expect(result.code).toBe(1);
    expect(result.out).toContain('var(--text-9) is not declared in tokens.css');
  });

  it('rejects a second declaration of a shared entry keyframe', () => {
    // R35(b), and the half nothing else can catch: a duplicate `@keyframes` does not error, the
    // later body silently wins, and a screen-local copy is invisible to motion.css's tier clamp.
    const result = gateOver('@keyframes viewIn { from { opacity: 0 } to { opacity: 1 } }\n');
    expect(result.code).toBe(1);
    expect(result.out).toContain('@keyframes viewIn is declared 2 times');
    expect(result.out).toContain('belongs in styles/base.css');
  });
});

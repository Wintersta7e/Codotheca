import { readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';

/**
 * **`AC-P3-33-5`** — §33.4's scope: the five material layers mount on the **opened hero and
 * nowhere else**. Never on the grid, in Peek, in the list, in the quick-switch palette, in
 * triage or on the Amnesty card — the same scope roasting has.
 *
 * **This is also the performance answer**: five extra elements on the one card on screen, never
 * five on each of 140 mounted tiles.
 *
 * The mechanism is a prop, so the audit is over the *source*: `CardPlate` renders whatever
 * `decay` it is handed, and only `HeroTile` hands it one. A surface that forgets to pass one
 * renders nothing, which is the correct default — and a surface that starts passing one is what
 * this catches.
 *
 * It lives in the **node** project and walks with `node:fs`, like `app/test/roastScope.test.ts`:
 * the same audit written as an `import.meta.glob` in the dom project raw-loads the whole renderer
 * tree and times out under the full parallel run.
 */
const appDir = fileURLToPath(new URL('..', import.meta.url));
const rendererSrc = join(appDir, 'src/renderer');

/**
 * The four files §33.4 permits, and what each is permitted to do.
 *
 * `decay/` declares the stack; `CardPlate` owns the slot; `Card` and `HeroFrame` pass it through;
 * `HeroTile` is the one surface that supplies one. **Every other file is a surface**, and a
 * surface naming `DecayStack` or passing `decay=` is a second one.
 */
const ALLOWED: ReadonlyArray<readonly [string, string]> = [
  ['src/renderer/card/CardPlate.tsx', 'declares the slot and renders whatever it is handed'],
  ['src/renderer/card/Card.tsx', 'passes it through; no band gains an element'],
  ['src/renderer/card/HeroFrame.tsx', 'hands the decoded scene hash to the callers callback'],
  ['src/renderer/project/hero/HeroTile.tsx', 'the ONE surface that supplies a stack'],
];

function sources(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    let isDir = false;
    try {
      isDir = statSync(path).isDirectory();
    } catch {
      // A probe another gate planted can vanish between the walk and the stat.
      continue;
    }
    if (isDir) sources(path, out);
    else if (/\.tsx?$/u.test(entry) && !/\.test\.tsx?$/u.test(entry)) out.push(path);
  }
  return out;
}

describe('the decay stack mounts on the opened hero and nowhere else', () => {
  it('ac_p3_33_5 finds no decay mount on any other surface, over a printed count', () => {
    const allowed = new Set(ALLOWED.map(([path]) => path));
    let scanned = 0;
    const offenders: string[] = [];
    for (const file of sources(rendererSrc)) {
      const text = readScannedFile(file);
      // Skipped BEFORE it is counted, so the guard below keeps meaning what it says.
      if (text === null) continue;
      scanned += 1;
      const name = relative(appDir, file).replace(/\\/gu, '/');
      // The decay module declares the component; it is not a surface that mounts one.
      if (name.startsWith('src/renderer/decay/')) continue;
      if (allowed.has(name)) continue;
      const code = text.replace(/\/\*[\s\S]*?\*\//gu, '').replace(/^[ \t]*\/\/.*$/gmu, '');
      if (/\bDecayStack\b/u.test(code) || /\bdecay=\{/u.test(code)) offenders.push(name);
    }
    console.error(`AC-P3-33-5: ${String(scanned)} renderer surfaces scanned`);
    expect(scanned, 'the surface scan read nothing at all').toBeGreaterThan(20);
    expect(offenders, 'a surface other than the opened hero mounts the decay stack').toEqual([]);
  });

  it('ac_p3_33_5 keeps the allowlist honest — every entry still does what it says', () => {
    // A stale allowlist reads as a live exemption and covers the next author who adds a mount.
    for (const [path, why] of ALLOWED) {
      const text = readScannedFile(join(appDir, path));
      expect(text, `${path} is on the allowlist and does not exist`).not.toBeNull();
      expect(
        /\bdecay\b/u.test(text ?? ''),
        `${path} is allowed because it ${why}, and it no longer names decay at all`,
      ).toBe(true);
    }
  });
});

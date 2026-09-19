import { existsSync, readdirSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';

/**
 * **`AC-P3-33-3`, the renderer half.** The renderer receives **resolved numbers** from
 * `health.weathering` and converts them to percentages. It never parses `scene_json`.
 *
 * That is the boundary §33.1 draws: the core resolves *which* vent is dusted from the geometry
 * sidecar, and the renderer positions percentages and counts open items. A second parser in the
 * renderer would be a second resolver in the product, and the two would drift on the first
 * change to the scene document.
 *
 * Both halves run: the **source** tree, which is always present, and the **built** bundle when
 * `npm run build:app` has produced one. The built half prints its byte count and fails at zero;
 * the source half prints its file count and fails at zero.
 */
const appDir = fileURLToPath(new URL('..', import.meta.url));
const rendererSrc = join(appDir, 'src/renderer');
const bundleDir = join(appDir, 'out/renderer');

/** The shapes that would mean a scene document is being read apart in the renderer. */
const FORBIDDEN = [/\bscene_json\b/u, /\bsceneJson\b/u, /\bcanonical_json\b/u];

/**
 * **Comments are stripped before the scan, and that is the half that keeps the audit honest.**
 *
 * `decay/layers.ts` opens by stating *the renderer never parses `scene_json`* — it documents the
 * boundary it implements. A text match over the raw file fails a correct tree, which is the
 * recorded *grepping a declaration matches prose about it* trap that has already produced two
 * wrong rulings here. What is asserted is the absence of an **identifier in code**.
 */
function stripComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//gu, '').replace(/^[ \t]*\/\/.*$/gmu, '');
}

function walk(dir: string, keep: (name: string) => boolean): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);
    let isDir = false;
    try {
      isDir = statSync(full).isDirectory();
    } catch {
      // A probe another gate planted can vanish between the walk and the stat.
      continue;
    }
    if (isDir) out.push(...walk(full, keep));
    else if (keep(entry)) out.push(full);
  }
  return out;
}

describe('the renderer parses no scene document', () => {
  it('finds no scene_json anywhere in the renderer source, over a non-zero file count', () => {
    const files = walk(rendererSrc, (n) => /\.(?:tsx?|css)$/u.test(n));
    let scanned = 0;
    const offenders: string[] = [];
    for (const file of files) {
      const text = readScannedFile(file);
      // Skipped BEFORE it is counted, so the guard below keeps meaning what it says.
      if (text === null) continue;
      scanned += 1;
      const code = stripComments(text);
      if (FORBIDDEN.some((pattern) => pattern.test(code))) offenders.push(relative(appDir, file));
    }
    console.error(`AC-P3-33-3: ${String(scanned)} renderer source file(s) scanned`);
    expect(scanned, 'the walk read no renderer source at all').toBeGreaterThan(0);
    expect(offenders, 'the renderer must receive resolved anchors, never a scene document').toEqual(
      [],
    );
  });

  it('finds no scene_json in the built renderer bundle, over a non-zero byte count', () => {
    if (!existsSync(bundleDir)) {
      // `npm run build:app` has not run in this tree. The source half above still gates, and
      // `npm run check:bundle` is what makes the built half reachable in the full gate.
      console.error('AC-P3-33-3: app/out/renderer absent — build:app has not run in this tree');
      return;
    }
    const files = walk(bundleDir, (n) => /\.(?:js|css|html)$/u.test(n));
    let bytes = 0;
    const offenders: string[] = [];
    for (const file of files) {
      const text = readScannedFile(file);
      if (text === null) continue;
      bytes += text.length;
      if (FORBIDDEN.some((pattern) => pattern.test(text))) offenders.push(relative(appDir, file));
    }
    console.error(`AC-P3-33-3: ${String(bytes)} bundle byte(s) scanned`);
    expect(bytes, 'the bundle walk read no bytes at all').toBeGreaterThan(0);
    expect(offenders).toEqual([]);
  });
});

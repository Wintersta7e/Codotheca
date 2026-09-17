import { readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from 'vitest';

import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';

const RENDERER = fileURLToPath(new URL('../src/renderer', import.meta.url));

/**
 * Every `.ts`/`.tsx` file under the renderer.
 *
 * **Reads through `readScannedFile`**, which skips a vanished file *before counting it*: other
 * gates plant probe files in these same directories and vitest runs the node project in
 * parallel, so an unguarded read throws a raw ENOENT and takes the whole suite red. Counting a
 * file that is no longer there would make the "scanned nothing" guard below stop meaning what it
 * says.
 */
function rendererSources(dir = RENDERER): { path: string; text: string }[] {
  const out: { path: string; text: string }[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...rendererSources(path));
      continue;
    }
    if (!/\.tsx?$/u.test(entry.name)) continue;
    const text: string | null = readScannedFile(path);
    if (text === null) continue;
    out.push({ path, text });
  }
  return out;
}

/**
 * §24.3d: Install is offered in **exactly two places**, both the slot Play occupies on a cloned
 * project — the blueprint card's primary action slot and the project page's left rail.
 *
 * Its own tests and the component itself do not count as mount points.
 */
test('InstallControl is mounted in exactly two places and no third', () => {
  const sources = rendererSources();
  // A gate whose passing run scans zero files is a failing gate.
  expect(sources.length, 'the renderer scan read nothing').toBeGreaterThan(50);

  const importers = sources
    .filter(({ path }) => !path.includes(`${'install'}${'/'}InstallControl`))
    .filter(({ path }) => !path.endsWith('.test.ts') && !path.endsWith('.test.tsx'))
    .filter(({ text }) => /\bInstallControl\b/u.test(text))
    .map(({ path }) => path.slice(RENDERER.length + 1).replaceAll('\\', '/'));

  expect(
    importers.sort(),
    `Install is offered in exactly two places; found ${importers.length}`,
  ).toEqual(['card/ProjectCard.tsx', 'project/rail/Rail.tsx']);
});

/**
 * [p2-24b] §24.5: Uninstall occupies **one** slot — the project page's left rail. It appears on no
 * card, in no Peek, in no list, in no palette row and in no triage surface, and the way that claim
 * dies is a second mount point nobody notices.
 *
 * `\bInstallControl\b` above does **not** match `UninstallControl` — `n` and `I` are both word
 * characters, so there is no boundary between them. The two gates are independent, which is the
 * R15 point: the names share four letters and nothing else.
 */
test('UninstallControl is mounted in exactly one place and no second', () => {
  const sources = rendererSources();
  expect(sources.length, 'the renderer scan read nothing').toBeGreaterThan(50);

  const importers = sources
    .filter(({ path }) => !path.includes(`${'uninstall'}${'/'}UninstallControl`))
    .filter(({ path }) => !path.endsWith('.test.ts') && !path.endsWith('.test.tsx'))
    .filter(({ text }) => /\bUninstallControl\b/u.test(text))
    .map(({ path }) => path.slice(RENDERER.length + 1).replaceAll('\\', '/'));

  expect(
    importers.sort(),
    `Uninstall is offered in exactly one place; found ${importers.length}`,
  ).toEqual(['project/rail/Rail.tsx']);
});

/**
 * GA39: the renderer's reload and the core's `projects` topic snapshot each make the same eight
 * `projects.list` reads, one per subset of the flags §8.0b's base set drops. One list stated in
 * two languages drifts, so this reads the Rust side and holds the two equal.
 */
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from 'vitest';

import { readScannedFile } from '../../scripts/lib/read-scanned.mjs';
import { LIBRARY_READS } from '../src/renderer/app/libraryReads';

const REPO = fileURLToPath(new URL('../..', import.meta.url));

test('GA39: the core snapshot and the renderer reload make the same library reads', () => {
  const source = readScannedFile(join(REPO, 'core/src/assembly/mod.rs'));
  expect(source).not.toBeNull();
  const block = /const PROJECTS_SNAPSHOT_READS[^=]*=\s*\[([\s\S]*?)\];/u.exec(source ?? '');
  expect(block, 'PROJECTS_SNAPSHOT_READS not found').not.toBeNull();
  const core = [...(block?.[1] ?? '').matchAll(/None|Some\("([^"]*)"\)/gu)].map((m) =>
    m[0] === 'None' ? null : (m[1] ?? ''),
  );
  console.error(
    `core reads ${String(core.length)}, renderer reads ${String(LIBRARY_READS.length)}`,
  );
  expect(core).toHaveLength(8);
  expect(core).toEqual([...LIBRARY_READS]);
});

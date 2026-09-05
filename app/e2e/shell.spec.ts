import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { _electron as electron, expect, test } from '@playwright/test';

// Playwright transpiles specs to CommonJS, so `import.meta` is not available here.
const appDir = path.resolve(__dirname, '..');

test('the shell opens exactly one window and the renderer sees no Node', async () => {
  // A fresh userData and data directory, for the same reason `mount.spec.ts` takes one and for a
  // sharper one: without it this launches the **real** app against the developer's own profile,
  // and the core it starts opens and migrates the real `index.db` there. On 2026-09-05 a run of
  // this suite carried a machine's live index from schema 7 to 8, after which a build without the
  // newer migration refuses to open it — correctly, by §14. A test may not write to the data
  // directory a person's library lives in.
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-shell-'));
  const app = await electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, CODOTHECA_DATA_DIR: userData },
  });
  const window = await app.firstWindow();
  await window.waitForLoadState('domcontentloaded');

  expect(app.windows()).toHaveLength(1);
  await expect.poll(() => window.title()).toBe('Codotheca');

  // Context isolation and the sandbox, asserted rather than assumed: the renderer may never
  // originate a filesystem path or an executable, and it cannot if it cannot reach Node.
  const reachable = await window.evaluate(() => ({
    process: typeof (globalThis as { process?: unknown }).process,
    require: typeof (globalThis as { require?: unknown }).require,
  }));
  expect(reachable).toEqual({ process: 'undefined', require: 'undefined' });

  await app.close();
});

import * as path from 'node:path';

import { _electron as electron, expect, test } from '@playwright/test';

// Playwright transpiles specs to CommonJS, so `import.meta` is not available here.
const appDir = path.resolve(__dirname, '..');

test('the shell opens exactly one window and the renderer sees no Node', async () => {
  const app = await electron.launch({ args: ['.'], cwd: appDir });
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

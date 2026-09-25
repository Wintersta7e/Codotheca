import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { _electron as electron, expect, test, type ElectronApplication } from '@playwright/test';

// Playwright transpiles specs to CommonJS, so `import.meta` is not available here.
const appDir = path.resolve(__dirname, '..');

/** Well inside the test's 60 s, so a stalled quit reports itself instead of the timeout. */
const CLOSE_BOUND_MS = 20_000;

/**
 * Every process that could be holding the app open, by name. A hosted Windows runner timed this
 * test out twice with nothing else to go on, and the same quit measured on a workstation left
 * nothing behind; listing by name rather than by descent still finds a child whose parent is gone.
 */
function processListing(): string {
  try {
    return process.platform === 'win32'
      ? execFileSync(
          'powershell.exe',
          [
            '-NoProfile',
            '-Command',
            "Get-CimInstance Win32_Process | Where-Object { $_.Name -match 'codotheca|electron|git' } | " +
              'ForEach-Object { "$($_.ProcessId) <- $($_.ParentProcessId) $($_.Name) $($_.CommandLine)" }',
          ],
          { encoding: 'utf8' },
        )
      : execFileSync('ps', ['-C', 'codotheca-core,electron,git', '-o', 'pid,ppid,etime,args'], {
          encoding: 'utf8',
        });
  } catch (error) {
    return `no process listing: ${String(error)}`;
  }
}

/** `app.close()`, bounded; on expiry the log and the process listing go beside the test output. */
async function closeWithin(app: ElectronApplication, userData: string): Promise<void> {
  let timer: NodeJS.Timeout | undefined;
  const expired = new Promise<'expired'>((resolve) => {
    timer = setTimeout(() => {
      resolve('expired');
    }, CLOSE_BOUND_MS);
  });
  const outcome = await Promise.race([app.close().then(() => 'closed' as const), expired]);
  clearTimeout(timer);
  if (outcome === 'closed') return;
  writeFileSync(test.info().outputPath('processes.txt'), processListing(), 'utf8');
  const log = path.join(userData, 'logs', 'codotheca.log');
  if (existsSync(log)) copyFileSync(log, test.info().outputPath('codotheca.log'));
  app.process().kill();
  throw new Error(`app.close() did not return within ${String(CLOSE_BOUND_MS)} ms`);
}

test('the shell opens exactly one window and the renderer sees no Node', async () => {
  // A fresh userData and data directory, for the same reason `mount.spec.ts` takes one and for a
  // sharper one: without it this launches the **real** app against the developer's own profile,
  // and the core it starts opens and migrates the real `index.db` there. On 2026-09-05 a run of
  // this suite carried a machine's live index from schema 7 to 8, after which a build without the
  // newer migration refuses to open it — correctly, by §14. A test may not write to the data
  // directory a person's library lives in.
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-shell-'));
  const started = Date.now();
  const step = (name: string): void => {
    console.warn(`shell.spec +${String(Date.now() - started)} ms: ${name}`);
  };
  const app = await electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, CODOTHECA_DATA_DIR: userData },
  });
  step('launched');
  const window = await app.firstWindow();
  await window.waitForLoadState('domcontentloaded');
  step('window loaded');

  expect(app.windows()).toHaveLength(1);
  await expect.poll(() => window.title()).toBe('Codotheca');
  step('titled');

  // Context isolation and the sandbox, asserted rather than assumed: the renderer may never
  // originate a filesystem path or an executable, and it cannot if it cannot reach Node.
  const reachable = await window.evaluate(() => ({
    process: typeof (globalThis as { process?: unknown }).process,
    require: typeof (globalThis as { require?: unknown }).require,
  }));
  expect(reachable).toEqual({ process: 'undefined', require: 'undefined' });
  step('sandbox checked');

  await closeWithin(app, userData);
  step('closed');
});

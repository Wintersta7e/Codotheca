import { execFile } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { _electron as electron, expect, test } from '@playwright/test';

// Playwright transpiles specs to CommonJS, so `import.meta` is not available here.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

/** The quit gate runs after the core lane joins, and joining needs the release core. */
const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);
const NO_CORE = 'the release core binary is not built, so the quit gate is never installed';

/** The Electron executable, found the way the `electron` package's own entry point finds it. */
function electronBinary(): string {
  const dir = path.dirname(require.resolve('electron/package.json'));
  return path.join(dir, 'dist', readFileSync(path.join(dir, 'path.txt'), 'utf8').trim());
}

function logOf(dataDir: string): string {
  const log = path.join(dataDir, 'logs', 'codotheca.log');
  return existsSync(log) ? readFileSync(log, 'utf8') : '';
}

/** The shell's own record of each quit step, in the order they ran. */
function quitSteps(dataDir: string): string[] {
  return logOf(dataDir)
    .split('\n')
    .flatMap((line) => {
      const step = / shell quit: (.*)$/u.exec(line);
      return step === null ? [] : [step[1] ?? ''];
    });
}

test('closing the window quits the app and every quit step succeeds', async () => {
  test.skip(!existsSync(coreBinary), NO_CORE);
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-quit-'));
  const app = await electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, CODOTHECA_DATA_DIR: userData },
  });
  await app.firstWindow();
  await expect.poll(() => logOf(userData), { timeout: 30_000 }).toMatch(/core lane: /u);

  // The user's close, not `app.quit()`: the window is destroyed before the quit steps run.
  const closed = app.waitForEvent('close', { timeout: 20_000 });
  await app.evaluate(({ BrowserWindow }) => {
    BrowserWindow.getAllWindows()[0]?.close();
  });
  await closed;

  expect(quitSteps(userData)).toEqual(['shortcut-release released', 'core-shutdown exited']);
});

test('the release launch script asks the real app to quit, and it does', async () => {
  test.skip(!existsSync(coreBinary), NO_CORE);
  test.setTimeout(180_000);
  const dataDir = mkdtempSync(path.join(tmpdir(), 'codotheca-launch-'));
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-launch-profile-'));
  const kind = process.platform === 'win32' ? 'nsis' : 'deb';
  // A hosted Linux runner restricts user namespaces, and the development Electron's
  // `chrome-sandbox` is not root-owned, so Chromium aborts before the app starts. What is under
  // test is the quit request; the packaged builds' sandbox is the release launch job's.
  const sandbox = process.platform === 'linux' ? ['--no-sandbox'] : [];
  const stdout = await new Promise<string>((resolve, reject) => {
    execFile(
      process.execPath,
      [
        path.join(repoRoot, 'scripts', 'launch-artifact.mjs'),
        ...['--kind', kind, '--exe', electronBinary(), '--data-dir', dataDir],
        ...['--', appDir, `--user-data-dir=${userData}`, ...sandbox],
      ],
      { encoding: 'utf8', timeout: 170_000 },
      (error, out, err) => {
        if (error === null) resolve(out);
        else reject(new Error(`${error.message}\n${out}\n${err}`));
      },
    );
  });

  expect(stdout).toMatch(/^quit: clean$/mu);
  expect(quitSteps(dataDir)).toEqual(['shortcut-release released', 'core-shutdown exited']);
});

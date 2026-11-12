import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { expect, test, type ElectronApplication, type Page } from '@playwright/test';

import { launchApp } from './launchApp.js';
import { FIXTURE_GIT_ENV, homeEnv } from './isolatedHome.js';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is not.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

/**
 * A project page opens at its top. The page has no scroller of its own — it scrolls the document
 * — so whatever scroll the previous page left behind is the next page's starting point unless the
 * page resets it. Only a real layout can show this: jsdom never clamps a scroll to its content.
 */
const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);

const EMAIL = 'e2e@example.invalid';
const NAMES = ['alpha', 'beta'] as const;

function seedHome(): string {
  const home = mkdtempSync(path.join(tmpdir(), 'codotheca-scroll-'));
  writeFileSync(
    path.join(home, '.gitconfig'),
    `[user]\n\tname = E2E\n\temail = ${EMAIL}\n`,
    'utf8',
  );
  const src = path.join(home, 'src');
  mkdirSync(src);
  for (const name of NAMES) {
    const dir = path.join(src, name);
    mkdirSync(dir);
    // A long README makes the overview taller than the window, so the document scrolls.
    const lines = Array.from({ length: 120 }, (_, i) => `Paragraph ${String(i + 1)} of ${name}.\n`);
    writeFileSync(path.join(dir, 'README.md'), `# ${name}\n\n${lines.join('\n')}`, 'utf8');
    execFileSync(
      'git',
      ['-c', 'user.name=E2E', '-c', `user.email=${EMAIL}`, 'init', '-q', '-b', 'main'],
      {
        cwd: dir,
        env: { ...process.env, ...FIXTURE_GIT_ENV },
        stdio: 'ignore',
      },
    );
    for (const args of [
      ['add', '.'],
      ['commit', '-qm', 'seed'],
    ]) {
      execFileSync('git', ['-c', 'user.name=E2E', '-c', `user.email=${EMAIL}`, ...args], {
        cwd: dir,
        env: { ...process.env, ...FIXTURE_GIT_ENV },
        stdio: 'ignore',
      });
    }
  }
  return home;
}

function launch(home: string): Promise<ElectronApplication> {
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-scroll-data-'));
  return launchApp({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, ...homeEnv(home), CODOTHECA_DATA_DIR: userData },
  });
}

/** The document's scroll and where the page's bar sits in the viewport. */
async function pageScroll(window: Page): Promise<{ scrollTop: number; barTop: number }> {
  // The page's entry gesture moves the bar for its first frames; read it once that has played.
  await window.waitForTimeout(2000);
  return window.evaluate(() => {
    const bar = document.querySelector('.cp-bar');
    return {
      scrollTop: document.scrollingElement?.scrollTop ?? -1,
      barTop: bar === null ? Number.NaN : bar.getBoundingClientRect().top,
    };
  });
}

async function scrollPageToEnd(window: Page): Promise<number> {
  return window.evaluate(() => {
    const doc = document.scrollingElement;
    if (doc === null) return -1;
    doc.scrollTop = doc.scrollHeight;
    return doc.scrollTop;
  });
}

/** Opens a page from the palette: Shift+Enter is the page, Enter would be Play. */
async function openPage(window: Page, name: string): Promise<void> {
  await window.keyboard.press('Alt+Space');
  await window.keyboard.type(name);
  await window.keyboard.press('Shift+Enter');
  await window.waitForSelector('[data-testid="cp-tabpanel"]', { timeout: 60_000 });
  await expect(window.locator('[data-testid="cp-page"] h1')).toContainText(name, {
    timeout: 60_000,
  });
}

test('a project page opens at its top, whatever the last page was scrolled to', async () => {
  test.skip(
    !existsSync(coreBinary),
    'the release core binary is not built, so there is no core to scan with',
  );
  test.setTimeout(240_000);

  const home = seedHome();
  const app = await launch(home);
  try {
    const window = await app.firstWindow();
    await window.waitForLoadState('domcontentloaded');
    // Short enough that the overview overflows it; `scrollPageToEnd` below fails if it does not.
    await app.evaluate(({ BrowserWindow }) => {
      BrowserWindow.getAllWindows()[0]?.setSize(1200, 520);
    });

    await window.waitForSelector('.cdt-fr-view--roots', { timeout: 60_000 });
    await window.getByRole('button', { name: 'DIG', exact: true }).click();
    const goOn = window.getByRole('button', { name: 'GO ON', exact: true });
    await goOn.waitFor({ state: 'visible', timeout: 150_000 });
    await goOn.click();
    const notNow = window.getByRole('button', { name: 'NOT NOW', exact: true });
    await notNow.waitFor({ state: 'visible', timeout: 60_000 });
    await notNow.click();
    await expect(window.locator('.cdt-card')).toHaveCount(2, { timeout: 60_000 });

    // Shelf → page A through the palette, then scrolled to its end.
    await openPage(window, 'alpha');
    expect(await scrollPageToEnd(window)).toBeGreaterThan(0);

    // Page A → page B: the page view stays mounted and only its project changes. Unfixed, B
    // opened 540px down with its bar off screen.
    await openPage(window, 'beta');
    const viaPalette = await pageScroll(window);

    // Page B scrolled → Esc to the shelf → page A. The shelf fits the window, so the document
    // clamps to 0 there and this path opened at the top even before the fix.
    expect(await scrollPageToEnd(window)).toBeGreaterThan(0);
    await window.keyboard.press('Escape');
    await expect(window.locator('[data-testid="cp-page"]')).toHaveCount(0, { timeout: 10_000 });
    await expect(window.locator('.cdt-card')).toHaveCount(2);
    await openPage(window, 'alpha');
    const viaShelf = await pageScroll(window);

    expect(viaPalette).toEqual({ scrollTop: 0, barTop: 0 });
    expect(viaShelf).toEqual({ scrollTop: 0, barTop: 0 });
  } finally {
    await app.close();
  }
});

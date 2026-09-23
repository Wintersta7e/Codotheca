import { execFileSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import {
  _electron as electron,
  expect,
  test,
  type ElectronApplication,
  type Page,
} from '@playwright/test';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

/**
 * **§8.5's hero, measured in a real window.** jsdom lays nothing out, so the unit test can only
 * assert that the hero resolves `aspect-ratio: 2/3`. At `auto` the hero measured 268×2 in the
 * built app: every layer in it is positioned, so the plate — and the decay layers and the
 * restoration surge inside it — drew into a box with no height, and no unit test could see it.
 */
const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);

const EMAIL = 'e2e@example.invalid';

/** One repository and no remote. No real project; nothing on the network is reached. */
function seedHome(): string {
  const home = mkdtempSync(path.join(tmpdir(), 'codotheca-hero-'));
  writeFileSync(
    path.join(home, '.gitconfig'),
    `[user]\n\tname = E2E\n\temail = ${EMAIL}\n`,
    'utf8',
  );
  const dir = path.join(home, 'src', 'shaped');
  mkdirSync(dir, { recursive: true });
  writeFileSync(path.join(dir, 'README.md'), '# shaped\n', 'utf8');
  const git = (...args: string[]): void => {
    execFileSync('git', ['-c', 'user.name=E2E', '-c', `user.email=${EMAIL}`, ...args], {
      cwd: dir,
      stdio: 'ignore',
    });
  };
  git('init', '-q', '-b', 'main');
  git('add', '.');
  git('commit', '-qm', 'seed');
  return home;
}

function launch(home: string, userData: string): Promise<ElectronApplication> {
  return electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, HOME: home, USERPROFILE: home, CODOTHECA_DATA_DIR: userData },
  });
}

/**
 * A stalled first run leaves no evidence on a CI runner once the window closes, so the screen,
 * the visible text and the shell's log are kept beside the test's output, which CI uploads.
 */
async function keepEvidence(window: Page, userData: string): Promise<void> {
  await window
    .screenshot({ path: test.info().outputPath('stall.png') })
    .catch((e: unknown) => console.warn(`no stall screenshot: ${String(e)}`));
  const text = await window
    .evaluate(() => document.body.innerText)
    .catch((e: unknown) => `no body text: ${String(e)}`);
  writeFileSync(test.info().outputPath('stall.txt'), text, 'utf8');
  const log = path.join(userData, 'logs', 'codotheca.log');
  if (existsSync(log)) copyFileSync(log, test.info().outputPath('codotheca.log'));
  else console.warn(`no shell log at ${log}`);
}

test('the project page hero keeps its 268×402 box, and its plate fills it', async () => {
  test.skip(
    !existsSync(coreBinary),
    'the release core binary is not built, so there is no core to scan with',
  );
  test.setTimeout(240_000);

  const home = seedHome();
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-hero-data-'));
  const app = await launch(home, userData);
  try {
    const window = await app.firstWindow();
    await window.waitForLoadState('domcontentloaded');

    await window.waitForSelector('.cdt-fr-view--roots', { timeout: 60_000 });
    const corpusRow = window
      .locator('[data-testid="fr-root-row"][data-tickable="true"]')
      .filter({ hasText: home });
    await expect(corpusRow).toHaveCount(1);
    await window.getByRole('button', { name: 'DIG', exact: true }).click();
    const goOn = window.getByRole('button', { name: 'GO ON', exact: true });
    try {
      await goOn.waitFor({ state: 'visible', timeout: 150_000 });
    } catch (error) {
      await keepEvidence(window, userData);
      throw error;
    }
    await goOn.click();
    const notNow = window.getByRole('button', { name: 'NOT NOW', exact: true });
    await notNow.waitFor({ state: 'visible', timeout: 60_000 });
    await notNow.click();
    await expect(window.locator('.cdt-card')).toHaveCount(1, { timeout: 60_000 });

    // Shift-click opens the project page — the card's `onOpen`, as against `onActivate`.
    await window
      .locator('.cdt-card')
      .first()
      .click({ modifiers: ['Shift'] });
    await window.waitForSelector('[data-testid="cp-tabpanel"]', { timeout: 60_000 });

    const hero = window.locator(".cdt-card[data-surface='hero']");
    await expect(hero).toHaveCount(1);
    // The page enters on a slide and the hero on a scale, so a box read mid-entry measures the
    // animation. Read once it has held still across two frames apart.
    await expect
      .poll(
        async () => {
          const before = JSON.stringify(await hero.boundingBox());
          await window.waitForTimeout(150);
          return before === JSON.stringify(await hero.boundingBox());
        },
        { timeout: 15_000 },
      )
      .toBe(true);
    const card = await hero.boundingBox();
    const plate = await hero.locator('.cdt-plate').boundingBox();
    console.warn(`hero card ${JSON.stringify(card)}; hero plate ${JSON.stringify(plate)}`);
    if (card === null || plate === null) throw new Error('the hero or its plate has no box');

    // §8.5: the 268px column, at the tile's 2:3.
    expect(Math.round(card.width)).toBe(268);
    expect(card.height / card.width).toBeCloseTo(1.5, 1);
    // The plate is inset 1px by the frame's padding, and it is the box every layer draws into.
    expect(plate.height).toBeGreaterThan(card.height - 4);
    expect(plate.width).toBeGreaterThan(card.width - 4);
  } finally {
    await app.close();
  }
});

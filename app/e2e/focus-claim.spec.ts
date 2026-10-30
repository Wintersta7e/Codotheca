import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { _electron as electron, expect, test, type ElectronApplication } from '@playwright/test';

import { FOCUS_HEARTBEAT_SECS } from '../src/shared/sessionFocus.js';
import { FIXTURE_GIT_ENV, homeEnv } from './isolatedHome.js';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

/**
 * §39.3a end to end: a project page claims focus only after real input in it.
 *
 * jsdom cannot make a trusted event, so the unit tests can prove only the negative half. This
 * one drives a real window with a real click and reads the core's answer from the one place a
 * renderer→core request is observable here: the line the core writes to stderr on a change of
 * claim, which the shell merges into the rolling log under the run's data directory.
 */
const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);

const EMAIL = 'e2e@example.invalid';
const CLAIM_LINE = 'session.focus claim=';

/** One repository under `src`, so the shelf has exactly one tile to open. */
function seedHome(): string {
  const home = mkdtempSync(path.join(tmpdir(), 'codotheca-home-'));
  writeFileSync(
    path.join(home, '.gitconfig'),
    `[user]\n\tname = E2E\n\temail = ${EMAIL}\n`,
    'utf8',
  );
  const dir = path.join(home, 'src', 'alpha');
  mkdirSync(dir, { recursive: true });
  writeFileSync(path.join(dir, 'README.md'), '# alpha\n', 'utf8');
  const git = (...args: string[]): void => {
    execFileSync('git', ['-c', 'user.name=E2E', '-c', `user.email=${EMAIL}`, ...args], {
      cwd: dir,
      env: { ...process.env, ...FIXTURE_GIT_ENV },
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
    env: { ...process.env, ...homeEnv(home), CODOTHECA_DATA_DIR: userData },
  });
}

function claimLines(userData: string): string[] {
  const log = path.join(userData, 'logs', 'codotheca.log');
  if (!existsSync(log)) return [];
  return readFileSync(log, 'utf8')
    .split('\n')
    .filter((line) => line.includes(CLAIM_LINE));
}

test('a project page claims focus at the first trusted input, not when it opens', async () => {
  test.skip(
    !existsSync(coreBinary),
    'the release core binary is not built, so there is no core to report focus to',
  );
  test.setTimeout(240_000);

  const home = seedHome();
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-focus-data-'));
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
    await goOn.waitFor({ state: 'visible', timeout: 150_000 });
    await goOn.click();
    const notNow = window.getByRole('button', { name: 'NOT NOW', exact: true });
    await notNow.waitFor({ state: 'visible', timeout: 60_000 });
    await notNow.click();
    await expect(window.locator('.cdt-card')).toHaveCount(1, { timeout: 60_000 });

    // The id the claim must name, read from the core rather than assumed from a fresh index.
    const projectId = await window.evaluate(async () => {
      const bridge = (
        globalThis as unknown as {
          codotheca: { request: (name: string, args: unknown) => Promise<unknown> };
        }
      ).codotheca;
      const reply = (await bridge.request('projects.list', {
        query: null,
        sort: null,
        window: null,
      })) as { ok: boolean; value?: { rows: { id: number }[] } };
      return reply.value?.rows.map((row) => row.id) ?? [];
    });
    expect(projectId).toHaveLength(1);

    // Shift-click opens the project page — the card's `onOpen`, as against `onActivate`.
    await window
      .locator('.cdt-card')
      .first()
      .click({ modifiers: ['Shift'] });
    await window.waitForSelector('[data-testid="cp-tabpanel"]', { timeout: 60_000 });

    // A claim made on opening would be sent at once; a negative needs a window to be one.
    await window.waitForTimeout(2_000);
    expect(claimLines(userData)).toEqual([]);

    const target = await window.getByTestId('cp-bar-path').boundingBox();
    if (target === null) throw new Error('the page bar has no box to click');
    await window.mouse.click(target.x + target.width / 2, target.y + target.height / 2);

    await expect
      .poll(() => claimLines(userData), { timeout: FOCUS_HEARTBEAT_SECS * 1000 })
      .toEqual([
        expect.stringMatching(new RegExp(`session\\.focus claim=${String(projectId[0])}$`, 'u')),
      ]);
  } finally {
    await app.close();
  }
});

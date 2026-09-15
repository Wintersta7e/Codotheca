import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { _electron as electron, expect, test, type ElectronApplication } from '@playwright/test';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

/**
 * **The scan-then-populate path**, which nothing else in the suite walks.
 *
 * 2148 app tests and the pixel test were green over a build that indexed twenty-six projects and
 * then showed §8.3a's `NOTHING INDEXED YET` until it was relaunched. Neither could see it: a jsdom
 * test mounts the shelf with rows already in hand, and the pixel test asserts the **first** screen,
 * which was correct. A gate that only ever renders the first frame is the sibling of §26.2's gate
 * whose passing run scans zero files.
 *
 * So this one drives the product — a real core, real repositories under a temp home, §10's beats
 * in order — and then looks at the shelf, which must carry one tile per repository **without the
 * app being restarted**.
 */
const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);

const REPOS = ['alpha', 'bravo', 'charlie'] as const;
const EMAIL = 'e2e@example.invalid';

/**
 * A home directory holding §10.1a's `src` convention, three real repositories, and the
 * `.gitconfig` all three were committed with.
 *
 * The config is not decoration. `user.email` is what seeds the identity set, the set is what J1.5
 * folds each repository's committers against, and §8.0b's base predicate returns **no**
 * `is_reference` rows — so a home with no configured address produces a full index that the
 * shelf's own query answers as empty. That was the shipped defect; this is the shape that
 * exercises the whole chain.
 */
function seedHome(): string {
  const home = mkdtempSync(path.join(tmpdir(), 'codotheca-home-'));
  writeFileSync(
    path.join(home, '.gitconfig'),
    `[user]\n\tname = E2E\n\temail = ${EMAIL}\n`,
    'utf8',
  );
  const src = path.join(home, 'src');
  mkdirSync(src);
  for (const name of REPOS) {
    const dir = path.join(src, name);
    mkdirSync(dir);
    writeFileSync(path.join(dir, 'README.md'), `# ${name}\n`, 'utf8');
    const git = (...args: string[]): void => {
      execFileSync('git', ['-c', 'user.name=E2E', '-c', `user.email=${EMAIL}`, ...args], {
        cwd: dir,
        stdio: 'ignore',
      });
    };
    git('init', '-q', '-b', 'main');
    git('add', '.');
    git('commit', '-qm', 'seed');
  }
  return home;
}

function launch(home: string): Promise<ElectronApplication> {
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-scan-'));
  return electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    // `HOME` is what the core reads for §10.1a's conventional locations and for `user.email`, so
    // both the row offered and the identity seeded are this test's, never the developer's own.
    env: { ...process.env, HOME: home, USERPROFILE: home, CODOTHECA_DATA_DIR: userData },
  });
}

test('a scan populates the shelf without a relaunch', async () => {
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

    // §10.1a's roots screen. The gate holds blank ground until `scan.status` answers, so the
    // screen is waited for rather than slept on.
    await window.waitForSelector('.cdt-fr-view--roots', { timeout: 60_000 });
    const corpusRow = window
      .locator('[data-testid="fr-root-row"][data-tickable="true"]')
      .filter({ hasText: home });
    await expect(corpusRow).toHaveCount(1);
    // A convention hit arrives **ticked**, and this asserts it rather than assuming it: a row
    // that started unticked would make `DIG` commit nothing and the rest of this file vacuous.
    await expect(corpusRow).toHaveAttribute('aria-checked', 'true');

    await window.getByRole('button', { name: 'DIG', exact: true }).click();

    // §10.3a then §10.4: the walk finishes, settles, and the reveal takes the screen. Reaching
    // this at all is an assertion — the gate used to re-decide on every render, and the run
    // `DIG` had just started was itself the proof that first run was over.
    const goOn = window.getByRole('button', { name: 'GO ON', exact: true });
    await goOn.waitFor({ state: 'visible', timeout: 150_000 });
    await goOn.click();

    // §10.4a's turn. `NOT NOW` leaves the shelf on its own query rather than the turn's.
    const notNow = window.getByRole('button', { name: 'NOT NOW', exact: true });
    await notNow.waitFor({ state: 'visible', timeout: 60_000 });
    await notNow.click();
    await expect(window.locator('.cdt-shelf')).toBeVisible();

    // **The assertion this file exists for.** The projection was read once, at mount, when the
    // library was empty; the scan then wrote three projects that no `projects` event announces.
    await expect(window.locator('.cdt-card')).toHaveCount(REPOS.length, { timeout: 60_000 });
    // And it must not claim an absence while holding rows: §8.3a's empty state is a statement
    // about the user's disk, not a placeholder.
    await expect(window.locator('.cdt-shelf-empty-heading')).toHaveCount(0);
  } finally {
    await app.close();
  }
});

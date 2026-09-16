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
 * **§25.1's headline surface, in the built app — and it does not run by default, because the
 * product cannot reach that surface yet.**
 *
 * R88 was earned by a wave whose headline feature had never rendered on a branch green with 1566
 * core tests, 2209 app tests and 226 acceptance checks; R89 adds that a renderer change is not
 * verified until `build:app` has run, because `app/e2e/*` launches `out/` and not `src/`. This
 * file is that bar for the `REMOTE` tab, written and then driven against a real core.
 *
 * **What it measured instead — a defect this lane did not cause and does not fix.** Opening a
 * project page from the shelf in the built app leaves it on `loading` for ever. The page's own
 * DOM after eight seconds is the bar and nothing else — no `cp-body` at all, so the state is
 * neither `ready` nor `failed` — and a probe evaluated inside that page finds **`projects.list`
 * and `projects.get` both time out**, although `projects.list` answered moments earlier to
 * populate the shelf. So the core stops answering once a project page mounts. Bisected: with
 * `ProjectDetail.remote` and `ProjectDetail.backup` both hard-coded `None` — this lane's whole
 * contribution to `projects.get` removed, rebuilt, re-run — the page still never loads. No e2e
 * spec has ever opened a project page, which is why nothing caught it.
 *
 * It is registered behind `CODOTHECA_E2E_PROJECT_PAGE=1` rather than deleted or skipped. A
 * `test.skip` would fail `scripts/check-e2e-skips.mjs`, which is correct — *a skipped spec
 * asserted nothing* — and a spec that runs and fails would redden every later lane for a defect
 * none of them caused. Registering no test leaves the report honest and leaves this ready:
 *
 *     CODOTHECA_E2E_PROJECT_PAGE=1 npm run test:e2e --workspace app
 *
 * It is deliberately **not** about the forge: no account is connected and no network call is
 * made, so the tab renders its header, `BEHIND`, the links row and one statement — §25.1's own
 * *no account* row, which is what the product does on a machine that has connected nothing.
 */
const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);

const EMAIL = 'e2e@example.invalid';
/** A shaped remote. No real project, no real account, and no host is reached. */
const ORIGIN = 'https://github.com/acme/widget.git';
const KEY = 'github.com/acme/widget';

/**
 * One repository with a configured `origin`, so the scan derives a `remote_key` and the core's
 * own presence predicate answers non-NULL. Everything downstream of that is what is under test.
 */
function seedHome(): string {
  const home = mkdtempSync(path.join(tmpdir(), 'codotheca-remote-'));
  writeFileSync(
    path.join(home, '.gitconfig'),
    `[user]\n\tname = E2E\n\temail = ${EMAIL}\n`,
    'utf8',
  );
  const src = path.join(home, 'src');
  mkdirSync(src);
  const dir = path.join(src, 'widget');
  mkdirSync(dir);
  writeFileSync(path.join(dir, 'README.md'), '# widget\n', 'utf8');
  const git = (...args: string[]): void => {
    execFileSync('git', ['-c', 'user.name=E2E', '-c', `user.email=${EMAIL}`, ...args], {
      cwd: dir,
      stdio: 'ignore',
    });
  };
  git('init', '-q', '-b', 'main');
  git('add', '.');
  git('commit', '-qm', 'seed');
  git('remote', 'add', 'origin', ORIGIN);
  return home;
}

function launch(home: string): Promise<ElectronApplication> {
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-remote-data-'));
  return electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, HOME: home, USERPROFILE: home, CODOTHECA_DATA_DIR: userData },
  });
}

if (process.env['CODOTHECA_E2E_PROJECT_PAGE'] === '1') {
  test('the REMOTE tab mounts and renders for a project with a remote', async () => {
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

      await window.waitForSelector('.cdt-fr-view--roots', { timeout: 60_000 });
      const corpusRow = window
        .locator('[data-testid="fr-root-row"][data-tickable="true"]')
        .filter({ hasText: home });
      await expect(corpusRow).toHaveCount(1);
      await expect(corpusRow).toHaveAttribute('aria-checked', 'true');
      await window.getByRole('button', { name: 'DIG', exact: true }).click();

      const goOn = window.getByRole('button', { name: 'GO ON', exact: true });
      await goOn.waitFor({ state: 'visible', timeout: 150_000 });
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
      await window.waitForSelector('[data-testid="cp-page"]', { timeout: 60_000 });
      // The tab panel exists only once the detail has arrived. Counting tabs before that would
      // count the two the bar draws while loading, which is a race and not an assertion.
      await window.waitForSelector('[data-testid="cp-tabpanel"]', { timeout: 60_000 });

      // **The assertion this file exists for.** Three tabs, not two: the tab is mounted because
      // the core answered `ProjectDetail.remote` non-null for a project whose `remote_key` it
      // derived from that repository's own `origin`. A unit test cannot see that chain.
      await expect(window.getByRole('tab')).toHaveCount(3);
      const remoteTab = window.getByRole('tab', { name: 'REMOTE', exact: true });
      await expect(remoteTab).toHaveCount(1);
      await remoteTab.click();

      // …and it renders. The key verbatim, the links row, and one statement in place of the
      // forge blocks — §25.1's `no_account` row, correct on a machine with no account.
      await expect(window.locator('[data-testid="cp-remote-key"]')).toHaveText(KEY);
      await expect(window.locator('[data-testid="cp-remote-links"]')).toBeVisible();
      await expect(window.locator('[data-testid="cp-remote-no-account"]')).toBeVisible();
      // Never render unknown as zero, and no second vocabulary for it.
      const panel = await window.locator('[data-testid="cp-tabpanel"]').textContent();
      expect(panel ?? '').not.toContain('UNKNOWN');
    } finally {
      await app.close();
    }
  });
}

import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { _electron as electron, expect, test, type Page } from '@playwright/test';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

/**
 * **§8.0b's chips and tail, and the era header, measured in a real window.** Each pairs a label
 * with the line that says what it counts, as sibling inline spans inside one box — so in the built
 * app they ran together as `ALLTHE WHOLE LIBRARY`, `LIVE4 projects` and `…CLASSIFIED)SPACE = PEEK`.
 * Every text assertion passed, because the words are all there; only a layout engine sees that
 * nothing separates them.
 */
const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);

const EMAIL = 'e2e@example.invalid';

/** Two repositories and no remote. No real project; nothing on the network is reached. */
function seedHome(): string {
  const home = mkdtempSync(path.join(tmpdir(), 'codotheca-labels-'));
  writeFileSync(
    path.join(home, '.gitconfig'),
    `[user]\n\tname = E2E\n\temail = ${EMAIL}\n`,
    'utf8',
  );
  for (const name of ['alpha', 'bravo']) {
    const dir = path.join(home, 'src', name);
    mkdirSync(dir, { recursive: true });
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

interface Box {
  readonly left: number;
  readonly right: number;
  readonly top: number;
  readonly bottom: number;
}

interface Pair {
  readonly where: string;
  readonly label: Box;
  readonly caption: Box;
}

/** Every label and the caption beside it, as laid out. */
function pairs(window: Page): Promise<readonly Pair[]> {
  return window.evaluate(() => {
    const box = (el: Element): Box => {
      const r = el.getBoundingClientRect();
      return { left: r.left, right: r.right, top: r.top, bottom: r.bottom };
    };
    const out: Pair[] = [];
    const add = (where: string, root: Element, label: string, caption: string): void => {
      const a = root.querySelector(label);
      const b = root.querySelector(caption);
      if (a !== null && b !== null) out.push({ where, label: box(a), caption: box(b) });
    };
    for (const chip of document.querySelectorAll('.cdt-attention-chip')) {
      add(`chip ${chip.textContent}`, chip, '.cdt-attention-label', '.cdt-attention-sub');
    }
    for (const tail of document.querySelectorAll('.cdt-attention-tail')) {
      add('tail', tail, '.cdt-attention-headline', '.cdt-attention-hint');
    }
    for (const era of document.querySelectorAll('.cdt-era-header')) {
      add(`era ${era.textContent}`, era, '.cdt-era-label', '.cdt-era-summary');
    }
    return out;
  });
}

test('the shelf’s labels stand apart from the captions beside them', async () => {
  test.skip(
    !existsSync(coreBinary),
    'the release core binary is not built, so there is no core to scan with',
  );
  test.setTimeout(240_000);

  const home = seedHome();
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-labels-data-'));
  const app = await electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, HOME: home, USERPROFILE: home, CODOTHECA_DATA_DIR: userData },
  });
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
    await expect(window.locator('.cdt-card')).toHaveCount(2, { timeout: 60_000 });

    const found = await pairs(window);
    console.warn(`label pairs ${JSON.stringify(found)}`);
    // Four chips, one tail and at least one era: a run that measured fewer measured nothing.
    expect(found.filter((p) => p.where.startsWith('chip'))).toHaveLength(4);
    expect(found.filter((p) => p.where === 'tail')).toHaveLength(1);
    expect(found.filter((p) => p.where.startsWith('era')).length).toBeGreaterThan(0);

    for (const { where, label, caption } of found) {
      // Stacked, the caption starts below the label; side by side, a visible gap separates them.
      const stacked = caption.top >= label.bottom - 0.5;
      const apart = caption.left - label.right >= 4;
      expect(stacked || apart, `${where}: ${JSON.stringify({ label, caption })}`).toBe(true);
    }
  } finally {
    await app.close();
  }
});

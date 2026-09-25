import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
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
 * §38.7.3 — **confirming the identity set never lowers the shelf's classified figure.**
 *
 * The confirmation moves projects across the Reference line; it never makes a classified project
 * unclassified. The store half is `core/tests/identity_confirm_history.rs`; this file asserts the
 * figure the user reads, on the rendered headline, through a real core and real repositories.
 *
 * The second test replays an observation — the classified count reading 0 after a confirmation
 * made the moment the first card painted — in its own order, sampling the figure from before the
 * click until every project is classified, so the sequence shows whether the confirmation lowered
 * it or authorship had simply not settled yet.
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
const SAMPLE_MS = 250;

/** A home holding §10.1a's `src` convention, three repositories and the `.gitconfig` they share. */
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
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-confirm-'));
  return electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, HOME: home, USERPROFILE: home, CODOTHECA_DATA_DIR: userData },
  });
}

/** §10's beats, from the roots screen to the shelf. */
async function toShelf(window: Page, home: string): Promise<void> {
  await window.waitForLoadState('domcontentloaded');
  await window.waitForSelector('.cdt-fr-view--roots', { timeout: 60_000 });
  const corpusRow = window
    .locator('[data-testid="fr-root-row"][data-tickable="true"]')
    .filter({ hasText: home });
  // DIG commits the rows ticked when it is pressed, so the suggestion is waited for — under load
  // it can take longer than an assertion's default five seconds to land.
  await expect(corpusRow).toHaveCount(1, { timeout: 30_000 });
  await expect(corpusRow).toHaveAttribute('aria-checked', 'true');
  await window.getByRole('button', { name: 'DIG', exact: true }).click();
  const goOn = window.getByRole('button', { name: 'GO ON', exact: true });
  await goOn.waitFor({ state: 'visible', timeout: 150_000 });
  await goOn.click();
  const notNow = window.getByRole('button', { name: 'NOT NOW', exact: true });
  await notNow.waitFor({ state: 'visible', timeout: 60_000 });
  await notNow.click();
}

const HEADLINE = /^(\d+) OF (\d+) · .+ · (\d+) REFERENCE EXCLUDED(?: \(OF (\d+) CLASSIFIED\))?$/;

interface Headline {
  readonly text: string;
  /** The `(OF n CLASSIFIED)` qualifier's `n`, else `total + reference`: no qualifier is complete. */
  readonly classified: number;
  readonly complete: boolean;
}

/** The rendered headline, read once; `null` while none is painted. */
async function headline(window: Page): Promise<Headline | null> {
  const node = window.locator('.cdt-attention-headline');
  if ((await node.count()) === 0) return null;
  const text = ((await node.first().textContent()) ?? '').trim();
  const match = HEADLINE.exec(text);
  if (match === null) throw new Error(`the headline does not parse: ${JSON.stringify(text)}`);
  const [, , total, reference, qualified] = match;
  const complete = qualified === undefined;
  const classified = complete ? Number(total) + Number(reference) : Number(qualified);
  return { text, classified, complete };
}

async function waitForComplete(window: Page, timeoutMs: number): Promise<Headline> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const read = await headline(window);
    if (read?.complete === true) return read;
    if (Date.now() > deadline) {
      throw new Error(`classification never completed; last headline ${JSON.stringify(read)}`);
    }
    await window.waitForTimeout(SAMPLE_MS);
  }
}

test("AC-P4-38-21 the headline's classified figure is unchanged by the identity confirmation", async () => {
  test.skip(!existsSync(coreBinary), 'the release core binary is not built');
  test.setTimeout(240_000);

  const home = seedHome();
  const app = await launch(home);
  try {
    const window = await app.firstWindow();
    await toShelf(window, home);

    const settled = await waitForComplete(window, 120_000);
    // The card is still up: it stays until it is answered.
    const confirm = window.getByRole('button', { name: 'CONFIRM', exact: true });
    await confirm.waitFor({ state: 'visible', timeout: 30_000 });
    await confirm.click();
    await expect(confirm).toHaveCount(0, { timeout: 30_000 });

    const samples: string[] = [];
    const until = Date.now() + 5_000;
    while (Date.now() < until) {
      const read = await headline(window);
      samples.push(read?.text ?? '<no headline>');
      await window.waitForTimeout(SAMPLE_MS);
    }
    // eslint-disable-next-line no-console -- the recorded headline and the sample count are the evidence
    console.log(
      `AC-P4-38-21 recorded ${JSON.stringify(settled.text)}; ${String(samples.length)} samples`,
    );
    expect(samples.length).toBeGreaterThan(0);
    expect(samples.filter((s) => s !== settled.text)).toEqual([]);
  } finally {
    await app.close();
  }
});

test('the classified figure never falls across a confirmation made before authorship settles', async () => {
  test.skip(!existsSync(coreBinary), 'the release core binary is not built');
  test.setTimeout(240_000);

  const home = seedHome();
  const app = await launch(home);
  try {
    const window = await app.firstWindow();
    await toShelf(window, home);

    // The observation's own order: the confirmation made at the first painted card.
    await window.locator('.cdt-card').first().waitFor({ timeout: 60_000 });
    const sequence: string[] = [];
    const figures: number[] = [];
    const sample = async (label: string): Promise<Headline | null> => {
      const read = await headline(window);
      sequence.push(`${label}${read === null ? '-' : String(read.classified)}`);
      if (read !== null) figures.push(read.classified);
      return read;
    };
    await sample('before:');
    const confirm = window.getByRole('button', { name: 'CONFIRM', exact: true });
    await confirm.waitFor({ state: 'visible', timeout: 30_000 });
    await confirm.click();
    await sample('click:');

    // Until every project is classified, and never less than five seconds after the click: the
    // confirmation's own re-read lands after it, so a figure already complete at the click would
    // otherwise end the trace before a fall could show.
    const settleUntil = Date.now() + 5_000;
    const deadline = Date.now() + 120_000;
    let read = await sample('');
    while ((read?.complete !== true || Date.now() < settleUntil) && Date.now() < deadline) {
      await window.waitForTimeout(SAMPLE_MS);
      read = await sample('');
    }
    // eslint-disable-next-line no-console -- the sampled sequence is the trace this test exists for
    console.log(`the classified figure across the confirmation: ${sequence.join(' ')}`);
    const falls = figures.flatMap((figure, i) =>
      i > 0 && figure < (figures[i - 1] ?? figure)
        ? [`${String(figures[i - 1])}→${String(figure)}`]
        : [],
    );
    expect(falls, `the classified figure fell: ${sequence.join(' ')}`).toEqual([]);
    expect(read?.complete, 'classification never completed').toBe(true);
  } finally {
    await app.close();
  }
});

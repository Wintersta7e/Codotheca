import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

import { _electron as electron, type ElectronApplication, type Page } from '@playwright/test';

import { FIXTURE_GIT_ENV, homeEnv } from './isolatedHome.js';

/**
 * A real ledger for the motivation specs: a home of real repositories carrying back-dated
 * commits, the app launched over it with a release core, and the one fixture write no running
 * core can make — day one moved into the past.
 */

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

export const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);
export const NO_CORE = 'the release core binary is not built, so there is no core to rise through';

export const EMAIL = 'e2e@example.invalid';

/** `YYYY-MM-DD` in this machine's zone, which the core's clock reads too. */
export function localDate(daysBack: number): string {
  const day = new Date();
  day.setDate(day.getDate() - daysBack);
  const pad = (n: number): string => String(n).padStart(2, '0');
  return `${String(day.getFullYear())}-${pad(day.getMonth() + 1)}-${pad(day.getDate())}`;
}

function git(dir: string, args: readonly string[], env: Record<string, string> = {}): void {
  execFileSync('git', ['-c', 'user.name=E2E', '-c', `user.email=${EMAIL}`, ...args], {
    cwd: dir,
    env: { ...process.env, ...FIXTURE_GIT_ENV, ...env },
    stdio: 'ignore',
  });
}

/** A home holding §10.1a's `src` convention, `repos` real repositories and their `.gitconfig`. */
export function seedHome(repos: readonly string[]): string {
  const home = mkdtempSync(path.join(tmpdir(), 'codotheca-home-'));
  writeFileSync(
    path.join(home, '.gitconfig'),
    `[user]\n\tname = E2E\n\temail = ${EMAIL}\n`,
    'utf8',
  );
  const src = path.join(home, 'src');
  mkdirSync(src);
  for (const name of repos) {
    const dir = path.join(src, name);
    mkdirSync(dir);
    writeFileSync(path.join(dir, 'README.md'), `# ${name}\n`, 'utf8');
    git(dir, ['init', '-q', '-b', 'main']);
    git(dir, ['add', '.']);
    git(dir, ['commit', '-qm', 'seed']);
  }
  return home;
}

/** One commit dated `daysBack` days ago at local noon, so its commit-day is that date. */
export function commitBackdated(home: string, repo: string, daysBack: number): void {
  const dir = path.join(home, 'src', repo);
  const date = `${localDate(daysBack)}T12:00:00`;
  writeFileSync(path.join(dir, `day-${String(daysBack)}.txt`), `${date}\n`, 'utf8');
  git(dir, ['add', '.']);
  git(dir, ['commit', '-qm', `day ${String(daysBack)}`], {
    GIT_AUTHOR_DATE: date,
    GIT_COMMITTER_DATE: date,
  });
}

export function launch(home: string, userData: string): Promise<ElectronApplication> {
  return electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, ...homeEnv(home), CODOTHECA_DATA_DIR: userData },
  });
}

export async function openWindow(app: ElectronApplication): Promise<Page> {
  const window = await app.firstWindow();
  await window.waitForLoadState('domcontentloaded');
  return window;
}

/** The index's own file, where the core keeps it under the data directory. */
const indexPath = (userData: string): string => path.join(userData, 'index.db');

/**
 * Fixture state, written while no core runs: day one eight days back, so §39's rings are drawn
 * rather than waived. Day one is today on any new install and no clock can be moved under a
 * release core; the rings' colours and transitions are what the cases measure.
 */
export function backdateDayOne(userData: string): void {
  const db = new DatabaseSync(indexPath(userData));
  try {
    db.exec('PRAGMA busy_timeout = 10000');
    db.prepare(
      'INSERT INTO app_meta (k, v) VALUES (?, ?) ON CONFLICT (k) DO UPDATE SET v = excluded.v',
    ).run('motivation_day_one', localDate(8));
  } finally {
    db.close();
  }
}

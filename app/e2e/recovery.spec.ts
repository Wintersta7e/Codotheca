import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import {
  _electron as electron,
  expect,
  test,
  type ElectronApplication,
  type Page,
  type TestInfo,
} from '@playwright/test';

import { homeEnv } from './isolatedHome.js';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

/**
 * §11.2a's startup report windows, painted by the real shell against the release core, and
 * §48.7.1's chain from the `corrupt_index` window through REBUILD to a restored shelf.
 *
 * Each kind is planted by `codotheca-sidecar-fixture` — an index the core cannot open — and the
 * app is launched over it. jsdom renders these windows happily; what it cannot say is whether
 * the shell ever *showed* one, and a window that never leaves `starting` is a black frame with
 * every unit test green.
 */
const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);

const KINDS = ['corrupt', 'future', 'migration-failed'] as const;

const NO_CORE =
  'the release core binary is not built, so no report window can be planted against it';

let fixtureBinary = '';

/**
 * Builds the fixture once and asks cargo where it put it, so a `CARGO_TARGET_DIR` is honoured
 * rather than guessed. A no-op when the binary is fresh.
 */
function buildFixture(): string {
  const out = execFileSync(
    'cargo',
    [
      'build',
      '--manifest-path',
      path.join(repoRoot, 'core', 'Cargo.toml'),
      '--features',
      'testkit',
      '--bin',
      'codotheca-sidecar-fixture',
      '--message-format=json-render-diagnostics',
    ],
    {
      cwd: repoRoot,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
      stdio: ['ignore', 'pipe', 'inherit'],
    },
  );
  for (const line of out.split('\n')) {
    if (!line.startsWith('{')) continue;
    const message = JSON.parse(line) as {
      reason?: string;
      target?: { name?: string };
      executable?: string | null;
    };
    if (
      message.reason === 'compiler-artifact' &&
      message.target?.name === 'codotheca-sidecar-fixture' &&
      typeof message.executable === 'string'
    ) {
      return message.executable;
    }
  }
  throw new Error('cargo built no codotheca-sidecar-fixture executable');
}

test.beforeAll(() => {
  if (!existsSync(coreBinary)) return;
  // A cold testkit build compiles the whole crate; CI builds it in an earlier step.
  test.setTimeout(600_000);
  fixtureBinary = buildFixture();
});

/**
 * Launches the app over `dataDir`. With `home`, the core reads the user's git config there: at
 * every start it adds each `user.email` it finds to the identity set, so a run that read this
 * machine's would add an address the planted library never held.
 */
async function launch(dataDir: string, home?: string): Promise<ElectronApplication> {
  const isolated = home === undefined ? {} : homeEnv(home);
  return electron.launch({
    args: ['.', `--user-data-dir=${dataDir}`],
    cwd: appDir,
    env: { ...process.env, ...isolated, CODOTHECA_DATA_DIR: dataDir },
  });
}

function fixture(...args: string[]): void {
  execFileSync(fixtureBinary, args, { stdio: ['ignore', 'ignore', 'inherit'] });
}

/**
 * Waits for the report window's headline, then measures that it painted and answers its text.
 *
 * Attach is not paint: `fw-root` enters with `viewIn … both`, whose `from` is opacity 0, so a
 * frame taken the instant the headline exists is one colour. `mount.spec.ts` measured exactly
 * that on CI; the same bounded settle applies. "Not one colour" is measured against a frame of
 * the same window that IS one colour — an opaque layer over everything. A flat frame compresses
 * to almost nothing, so a painted one is several times larger; a golden image would prove
 * nothing about whether this run drew.
 */
async function paintedReport(window: Page, testInfo: TestInfo, name: string): Promise<string> {
  await window.waitForLoadState('domcontentloaded');
  await window.waitForSelector('[data-testid="fw-headline"]', { timeout: 30_000 });
  const settle = await window.evaluate(async () => {
    const root = document.querySelector('[data-testid="fw-root"]');
    if (root === null) return { animations: 0, opacity: 'no root' };
    const running = root.getAnimations({ subtree: true });
    await Promise.race([
      Promise.allSettled(running.map((a) => a.finished)),
      new Promise((resolve) => setTimeout(resolve, 5_000)),
    ]);
    await new Promise((resolve) => {
      requestAnimationFrame(() => {
        requestAnimationFrame(resolve);
      });
    });
    return { animations: running.length, opacity: getComputedStyle(root).opacity };
  });

  const painted = await window.screenshot();
  await window.evaluate(() => {
    const cover = document.createElement('div');
    cover.id = 'paint-probe';
    cover.style.cssText = 'position:fixed;inset:0;z-index:2147483647;background:#000';
    document.body.append(cover);
  });
  const blank = await window.screenshot();
  await window.evaluate(() => {
    document.getElementById('paint-probe')?.remove();
  });
  writeFileSync(testInfo.outputPath(`${name}-painted.png`), painted);
  writeFileSync(testInfo.outputPath(`${name}-blank.png`), blank);
  // eslint-disable-next-line no-console -- the measurement is the deliverable
  console.log(
    `${name}: painted=${String(painted.byteLength)} blank=${String(blank.byteLength)} ` +
      `animations=${String(settle.animations)} opacity=${settle.opacity}`,
  );
  expect(blank.byteLength).toBeGreaterThan(0);
  expect(painted.byteLength).toBeGreaterThan(blank.byteLength * 4);
  return (await window.locator('body').innerText()).toLowerCase();
}

// Playwright rejects a non-destructured first argument outright, so the empty pattern is the
// only way to reach `testInfo`.
test('AC-P4-48-13 every startup report window paints', async ({}, testInfo) => {
  test.skip(!existsSync(coreBinary), NO_CORE);
  test.setTimeout(KINDS.length * 60_000);

  for (const kind of KINDS) {
    await test.step(kind, async () => {
      const dataDir = mkdtempSync(path.join(tmpdir(), `codotheca-recovery-${kind}-`));
      fixture('plant', '--kind', kind, '--data-dir', dataDir);

      const app = await launch(dataDir);
      let text: string;
      try {
        text = await paintedReport(await app.firstWindow(), testInfo, kind);
      } finally {
        await app.close();
      }

      // §48.7.2: before REBUILD nothing has been moved, so the window may not say it was.
      if (kind === 'corrupt') expect(text).not.toContain('set aside');
    });
  }
});

/** What `plant --kind populated` wrote to `<data-dir>/fixture.json`. */
interface Planted {
  /** The files a rebuild sets aside, by name: the index, its journal pair and the sidecar. */
  readonly quarantinable: readonly string[];
  /** The sidecar's count keys and their planted values. */
  readonly counts: Readonly<Record<string, number>>;
  /** The sections a rebuild restores before any scan: every global and no-scan one. */
  readonly immediate: readonly string[];
  /** The repositories the shelf finds again, by directory name. */
  readonly shelf: readonly string[];
}

/** What `verify` wrote to `<data-dir>/verify.json`, from the index the app left. */
interface Verified {
  /** Each count key, as `[planted, after the app quit]`. */
  readonly counts: Readonly<Record<string, readonly [number, number]>>;
  /** Top-level rows of the planted document compared against the rebuilt one. */
  readonly compared: number;
  /** Every field the round trip changed, but for `generation` and `written_at`. */
  readonly differences: readonly string[];
  /** Records still waiting in the pending table. */
  readonly pending: number;
  /** Subjects whose rebuilt session count is not the planted one. */
  readonly sessionMismatch: readonly string[];
  /** Raise-only sections checked, and those whose restored value is below the planted one. */
  readonly raiseOnly: number;
  readonly lowered: readonly string[];
}

/** `name` → the `<t>` of its `<name>.corrupt-<t>` sibling, for each set-aside file. */
function quarantined(dataDir: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const entry of readdirSync(dataDir)) {
    const match = /^(.*)\.corrupt-(\d+)$/u.exec(entry);
    if (match?.[1] !== undefined && match[2] !== undefined) out[match[1]] = match[2];
  }
  return out;
}

test('AC-P4-48-12 the corrupt-index chain restores every registered section end to end', async ({}, testInfo) => {
  test.skip(!existsSync(coreBinary), NO_CORE);
  test.setTimeout(300_000);
  const dataDir = mkdtempSync(path.join(tmpdir(), 'codotheca-recovery-populated-'));
  const reposDir = mkdtempSync(path.join(tmpdir(), 'codotheca-recovery-repos-'));
  const homeDir = mkdtempSync(path.join(tmpdir(), 'codotheca-recovery-home-'));
  fixture('plant', '--kind', 'populated', '--data-dir', dataDir, '--repos', reposDir);
  const planted = JSON.parse(readFileSync(path.join(dataDir, 'fixture.json'), 'utf8')) as Planted;

  const app = await launch(dataDir, homeDir);
  const window = await app.firstWindow();
  const text = await paintedReport(window, testInfo, 'populated');
  // §48.7.2: before REBUILD nothing has been moved, so the window may not say it was.
  expect(text).not.toContain('set aside');
  expect(quarantined(dataDir)).toEqual({});

  await window.getByRole('button', { name: 'REBUILD' }).click();

  // The three files are moved and the sidecar copied, all under one quarantine time.
  await expect
    .poll(() => Object.keys(quarantined(dataDir)).sort(), { timeout: 60_000 })
    .toEqual([...planted.quarantinable].sort());
  expect(new Set(Object.values(quarantined(dataDir))).size).toBe(1);

  // The report is written before the core's run loop starts, so what it counts was restored
  // before the scan found anything: the global and no-scan sections need no scan.
  const report = JSON.parse(readFileSync(path.join(dataDir, 'rebuild-report.json'), 'utf8')) as {
    restored: Record<string, number>;
  };
  for (const section of planted.immediate) {
    expect(report.restored[section], `${section} restored before the scan`).toBeGreaterThan(0);
  }

  for (const name of planted.shelf) {
    await expect(window.locator('.cdt-card', { hasText: name })).toHaveCount(1, {
      timeout: 60_000,
    });
  }
  const notice = await window.locator('body').innerText();
  for (const name of planted.quarantinable) {
    expect(notice).toContain(`${name}.corrupt-`);
  }

  // The user's close, not `app.quit()`: the core exports on its clean shutdown.
  const closed = app.waitForEvent('close', { timeout: 20_000 });
  await app.evaluate(({ BrowserWindow }) => {
    BrowserWindow.getAllWindows()[0]?.close();
  });
  await closed;

  fixture('verify', '--data-dir', dataDir);
  const verified = JSON.parse(readFileSync(path.join(dataDir, 'verify.json'), 'utf8')) as Verified;
  for (const [key, [before, after]] of Object.entries(verified.counts)) {
    // eslint-disable-next-line no-console -- the per-section count is the deliverable
    console.log(`${key}: ${String(before)} planted, ${String(after)} after the rebuild`);
  }
  // eslint-disable-next-line no-console -- the measurement is the deliverable
  console.log(
    `rows compared: ${String(verified.compared)}; raise-only sections: ${String(verified.raiseOnly)}`,
  );
  expect(verified.differences).toEqual([]);
  expect(verified.pending).toBe(0);
  expect(verified.sessionMismatch).toEqual([]);
  expect(verified.lowered).toEqual([]);
  const restored = Object.values(verified.counts).reduce((sum, [, after]) => sum + after, 0);
  expect(restored, 'nothing was restored').toBeGreaterThan(0);
  expect(verified.compared).toBeGreaterThan(0);
});

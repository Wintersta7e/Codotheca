import { existsSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { _electron as electron, expect, test, type ElectronApplication } from '@playwright/test';

import { TOP_BAR_FLOOR_PX } from '../src/renderer/shelf/TopBar.js';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

/**
 * The first assertion in this repository about a **pixel**.
 *
 * Everything else is jsdom, which has no layout engine, no compositor and no paint — a suite can
 * be entirely green over a product that renders nothing, and for two sessions it was. This
 * launches the real app against an empty data directory and looks at what the window actually
 * contains.
 *
 * `resolveCoreBinary` points at `core/target/release/codotheca-core` in development, and the
 * first-run gate holds blank ground until the core answers `scan.status`. Without that binary
 * there is no screen to assert, so the test says so and skips rather than passing on an empty
 * window — a green run over a blank frame is the exact failure this file exists to end.
 */
const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);

async function launch(): Promise<ElectronApplication> {
  // A fresh userData every run: first run is exactly what this is looking at, and a directory
  // carrying yesterday's index would paint the shelf instead.
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-mount-'));
  return electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, CODOTHECA_DATA_DIR: userData },
  });
}

// Playwright rejects a non-destructured first argument outright, so the empty pattern is the
// only way to reach `testInfo`.
test('the app paints a real first-run screen', async ({}, testInfo) => {
  test.skip(
    !existsSync(coreBinary),
    'the release core binary is not built, so the first-run gate has nothing to wait on',
  );

  const app = await launch();
  const window = await app.firstWindow();
  await window.waitForLoadState('domcontentloaded');

  // The gate holds blank ground until `scan.status` lands, so the screen is what is waited for
  // rather than a fixed delay.
  await window.waitForSelector('.cdt-fr-view', { timeout: 30_000 });
  await expect
    .poll(() => window.locator('.cdt-fr-roots').count(), { timeout: 30_000 })
    .toBeGreaterThan(0);

  // The selector resolves on DOM attach, and attach is not paint. `.cdt-fr-view` carries
  // `animation: viewIn .3s both` (`firstrun/firstRun.css:53-58`) over
  // `@keyframes viewIn { from { opacity: 0 } … }` (`styles/base.css:51`), and `fill-mode: both`
  // holds that `from` state until the animation actually runs — so a capture taken the instant
  // the element exists is a screen at **opacity 0**, and every child of it is invisible while
  // still laying out and measuring normally. That is what CI captured: a frame of a single
  // colour, `--app-bg` `#07090b`, with `.cdt-fr-view`'s own `--surface-0` `#0a0d10` absent.
  // Wait for the entry animation to settle rather than for a duration, then let two frames
  // compose. Bounded, because an animation that never starts must fail on the paint assertion
  // with both PNGs on disk rather than hang here with none.
  const settle = await window.evaluate(async () => {
    const view = document.querySelector('.cdt-fr-view');
    if (view === null) return { animations: 0, opacity: 'no view' };
    const running = view.getAnimations({ subtree: true });
    await Promise.race([
      Promise.allSettled(running.map((a) => a.finished)),
      new Promise((resolve) => setTimeout(resolve, 5_000)),
    ]);
    await new Promise((resolve) => {
      requestAnimationFrame(() => {
        requestAnimationFrame(resolve);
      });
    });
    return { animations: running.length, opacity: getComputedStyle(view).opacity };
  });

  // 1. It painted. Compared against a blank frame **of the same window** rather than a golden
  //    image: a fixed reference would fail on any font, DPI or GPU difference and would prove
  //    nothing about whether anything was drawn. The blank frame is this window with one opaque
  //    layer over it, so both PNGs share size, scale factor and encoder.
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

  // Both frames are kept whatever happens. When this assertion fails on a machine nobody can
  // attach a debugger to, the byte counts alone cannot distinguish "the app drew nothing" from
  // "the capture returned an uncomposited surface", and those have opposite fixes.
  // Written through `outputPath`, never `attach({ body })` — a body-only attachment lives in the
  // reporter and never reaches the output directory a CI job can upload. Both frames are written
  // on a green run too; `test-results/` is gitignored.
  writeFileSync(testInfo.outputPath('painted.png'), painted);
  writeFileSync(testInfo.outputPath('blank.png'), blank);
  // eslint-disable-next-line no-console -- the measurement is the deliverable
  console.log(
    `paint probe: painted=${String(painted.byteLength)} blank=${String(blank.byteLength)} ` +
      `animations=${String(settle.animations)} opacity=${settle.opacity}`,
  );

  // A flat frame compresses to almost nothing and is still a valid non-empty PNG, which is why
  // "the screenshot exists" proves nothing at all.
  expect(blank.byteLength).toBeGreaterThan(0);
  expect(painted.byteLength).toBeGreaterThan(blank.byteLength * 4);

  // 2. §10.1's roots screen, by its accessible name rather than by a class.
  const heading = await window.locator('h1, [role="heading"]').first().textContent();
  expect(heading?.length ?? 0).toBeGreaterThan(0);

  // 3. §10.1: the word appears nowhere. Checked against a real render for the first time.
  const text = (await window.locator('body').innerText()).toLowerCase();
  expect(text).not.toContain('setup');

  // 4. The document has width, and the window is at or above the measured floor.
  // Playwright reports no viewport for an Electron window — it emulates none — so the window's
  // own reading is the measurement.
  const geometry = await window.evaluate(() => {
    // `window` in this file is Playwright's page handle, so the browser's own globals are
    // reached through `globalThis` — the same form `shell.spec.ts` uses.
    const view = globalThis as unknown as { innerWidth: number; innerHeight: number };
    return {
      scrollWidth: document.body.scrollWidth,
      innerWidth: view.innerWidth,
      innerHeight: view.innerHeight,
    };
  });
  expect(geometry.innerHeight).toBeGreaterThan(0);
  expect(geometry.scrollWidth).toBeGreaterThan(0);
  expect(geometry.innerWidth).toBeGreaterThanOrEqual(TOP_BAR_FLOOR_PX);

  await app.close();
});

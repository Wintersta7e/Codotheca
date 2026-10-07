import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { _electron as electron, expect, test, type ElectronApplication } from '@playwright/test';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

/**
 * §11.2a's startup report windows, painted by the real shell against the release core.
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

async function launch(dataDir: string): Promise<ElectronApplication> {
  return electron.launch({
    args: ['.', `--user-data-dir=${dataDir}`],
    cwd: appDir,
    env: { ...process.env, CODOTHECA_DATA_DIR: dataDir },
  });
}

// Playwright rejects a non-destructured first argument outright, so the empty pattern is the
// only way to reach `testInfo`.
test('AC-P4-48-13 every startup report window paints', async ({}, testInfo) => {
  test.skip(
    !existsSync(coreBinary),
    'the release core binary is not built, so no report window can be planted against it',
  );
  test.setTimeout(KINDS.length * 60_000);

  for (const kind of KINDS) {
    await test.step(kind, async () => {
      const dataDir = mkdtempSync(path.join(tmpdir(), `codotheca-recovery-${kind}-`));
      execFileSync(fixtureBinary, ['plant', '--kind', kind, '--data-dir', dataDir], {
        stdio: ['ignore', 'ignore', 'inherit'],
      });

      const app = await launch(dataDir);
      const window = await app.firstWindow();
      await window.waitForLoadState('domcontentloaded');
      await window.waitForSelector('[data-testid="fw-headline"]', { timeout: 30_000 });

      // Attach is not paint: `fw-root` enters with `viewIn … both`, whose `from` is opacity 0, so
      // a frame taken the instant the headline exists is one colour. `mount.spec.ts` measured
      // exactly that on CI; the same bounded settle applies.
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

      // "Not one colour", measured against a frame of the same window that IS one colour: an
      // opaque layer over everything. A flat frame compresses to almost nothing, so a painted one
      // is several times larger; a golden image would prove nothing about whether this run drew.
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
      writeFileSync(testInfo.outputPath(`${kind}-painted.png`), painted);
      writeFileSync(testInfo.outputPath(`${kind}-blank.png`), blank);
      // eslint-disable-next-line no-console -- the measurement is the deliverable
      console.log(
        `${kind}: painted=${String(painted.byteLength)} blank=${String(blank.byteLength)} ` +
          `animations=${String(settle.animations)} opacity=${settle.opacity}`,
      );

      const text = (await window.locator('body').innerText()).toLowerCase();
      await app.close();

      expect(blank.byteLength).toBeGreaterThan(0);
      expect(painted.byteLength).toBeGreaterThan(blank.byteLength * 4);
      // §48.7.2: before REBUILD nothing has been moved, so the window may not say it was.
      if (kind === 'corrupt') expect(text).not.toContain('set aside');
    });
  }
});

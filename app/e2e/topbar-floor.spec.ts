import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { _electron as electron, expect, test } from '@playwright/test';
import { build } from 'esbuild';

import { TOP_BAR_FLOOR_PX, TOP_BAR_HEIGHT_PX } from '../src/renderer/shelf/TopBar.js';
import { SHED_WIDTHS } from '../src/renderer/shelf/useShedLevel.js';
import { SORT_LABELS } from '../src/renderer/shelf/viewState.js';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const rendererDir = path.join(appDir, 'src/renderer');
const nodeModules = path.resolve(appDir, '..', 'node_modules');

/**
 * §8.0a states the bar's floor is **measured, not asserted** — the width at which the last shed
 * step still fits at the widest sort label — and this is that measurement. It fails when the
 * committed constant is wrong in either direction.
 *
 * It measures the real component and the real stylesheet in a real layout engine. The app's own
 * window is not used: nothing mounts the shelf into the product yet, so the harness opens its
 * own Electron window — the same Chromium the app ships — over markup rendered from `TopBar`
 * itself. Restating the bar's DOM as a fixture would be a second copy free to drift from the
 * component whose width is the thing being measured.
 */

/**
 * Every value the sort control can render, off the tracked §2.4 contract.
 *
 * ~~`const WIDEST_SORT = 'last_touched'`~~ **[p3]** The widest label is **derived by measuring
 * every variant**, never named. `Last touched` was the widest of three; `Needs attention` is
 * wider, and at shed level 2 the bar drops the `SORT` key and renders the value alone
 * (`TopBar.tsx:90-91`), so the value's width **is** the floor. A harness that keeps naming one
 * variant measures a floor the product does not have and reports it green (§35.6, §26.2).
 *
 * Read from the schema rather than from `SORT_KEYS`, so this and `viewState.test.ts`'s
 * `AC-P3-35-4` derive the same list from the same place.
 */
function sortVariants(): readonly string[] {
  const schema = JSON.parse(
    readFileSync(path.resolve(appDir, '..', 'protocol/schema/protocol.json'), 'utf8'),
  ) as { types: { SortKey: { variants: string[] } } };
  return schema.types.SortKey.variants;
}

/** The three faces the bar draws in. Fallback metrics are not the product's metrics, so the
 *  real faces are loaded from the files the renderer bundles. */
const FONT_SHEETS = [
  '@fontsource/rajdhani/latin-500.css',
  '@fontsource/rajdhani/latin-600.css',
  '@fontsource/rajdhani/latin-700.css',
  '@fontsource/barlow/latin-400.css',
  '@fontsource/barlow/latin-500.css',
  '@fontsource/barlow/latin-600.css',
  '@fontsource/jetbrains-mono/latin-400.css',
  '@fontsource/jetbrains-mono/latin-500.css',
  '@fontsource/jetbrains-mono/latin-700.css',
];

/**
 * The faces, inlined as data URLs.
 *
 * A `file://` reference would not load: fonts are CORS-restricted and a file document is its own
 * opaque origin, so the harness would silently fall back to a system face and measure a
 * different bar. The guard in the test asserts all nine really loaded, because that failure is
 * invisible in the numbers.
 */
function fontFaces(): string {
  return FONT_SHEETS.map((sheet) => {
    const file = path.join(nodeModules, sheet);
    const dir = path.dirname(file);
    return readFileSync(file, 'utf8')
      .replace(/,\s*url\(\.\/files\/[^)]+\)\s*format\('woff'\)/g, '')
      .replace(/url\(\.\/files\/([^)]+)\)/g, (_match, name: string) => {
        const bytes = readFileSync(path.join(dir, 'files', name)).toString('base64');
        return `url(data:font/woff2;base64,${bytes})`;
      });
  }).join('\n');
}

/**
 * The bar's markup at each shed level **× each sort variant**, rendered by the component itself.
 *
 * It cannot be rendered in this process: Playwright compiles every TypeScript file it loads with
 * its own JSX transform, which emits component-testing descriptors rather than React elements,
 * so `renderToStaticMarkup` on the imported component throws. The component is bundled from the
 * same sources the app builds and rendered in a child Node process instead.
 *
 * [p3] The child takes the variant list as an **argument** rather than a constant of its own, so
 * one list — the schema's — reaches both this file's derivation and the markup it measures.
 */
async function barMarkupPerLevel(
  variants: readonly string[],
): Promise<readonly (readonly string[])[]> {
  const dir = mkdtempSync(path.join(tmpdir(), 'cdt-bar-render-'));
  const entry = path.join(dir, 'render.ts');
  const bundle = path.join(dir, 'render.cjs');
  writeFileSync(
    entry,
    `import { createElement } from 'react';
     import { renderToStaticMarkup } from 'react-dom/server';
     import { parseQuery } from ${JSON.stringify(path.join(appDir, 'src/shared/query/parse.js'))};
     import { fieldModel } from ${JSON.stringify(path.join(rendererDir, 'shelf/QueryField.js'))};
     import { TopBar } from ${JSON.stringify(path.join(rendererDir, 'shelf/TopBar.js'))};
     import { SHED_WIDTHS } from ${JSON.stringify(path.join(rendererDir, 'shelf/useShedLevel.js'))};
     import { DEFAULT_SHELF_VIEW } from ${JSON.stringify(path.join(rendererDir, 'shelf/viewState.js'))};
     const noop = () => undefined;
     const variants = ${JSON.stringify(variants)};
     // One width per level: 2000 sheds nothing, and each threshold is the widest width at which
     // its own step engages. Each level carries one markup per sort variant.
     const perLevel = [2000, ...SHED_WIDTHS].map((barWidth) =>
       variants.map((sort) =>
         renderToStaticMarkup(
           createElement(TopBar, {
             view: { ...DEFAULT_SHELF_VIEW, sort },
             field: fieldModel('', parseQuery(''), []),
             scan: { running: false, foundRepos: 0 },
             barWidth,
             onQueryChange: noop, onSortChange: noop, onDensityChange: noop,
             onViewModeChange: noop, onScan: noop, onOpenScanSummary: noop,
             onOpenPalette: noop, onOpenSettings: noop,
           }),
         ),
       ),
     );
     process.stdout.write(JSON.stringify(perLevel));`,
  );

  await build({
    entryPoints: [entry],
    outfile: bundle,
    bundle: true,
    platform: 'node',
    format: 'cjs',
    jsx: 'automatic',
    absWorkingDir: appDir,
    // The entry lives in a temp directory, so the workspace's module root has to be named.
    nodePaths: [nodeModules],
    // React is bundled in, not left external: the bundle runs from a temp directory, which has
    // no node_modules to resolve it from.
    // The sources import `./x.js` where the file on disk is `x.ts` or `x.tsx` — the resolution
    // the app's bundler does and esbuild does not do on its own.
    plugins: [
      {
        name: 'ts-source-for-js-specifier',
        setup(build) {
          build.onResolve({ filter: /^\.{0,2}\/.*\.js$/ }, (args) => {
            const stem = path.resolve(args.resolveDir, args.path).slice(0, -'.js'.length);
            for (const ext of ['.ts', '.tsx']) {
              try {
                readFileSync(stem + ext);
                return { path: stem + ext };
              } catch {
                // not this extension; try the next
              }
            }
            return null;
          });
        },
      },
    ],
  });

  const rendered: unknown = JSON.parse(
    execFileSync(process.execPath, [bundle], { cwd: appDir, encoding: 'utf8' }),
  );
  const levels = 1 + SHED_WIDTHS.length;
  if (
    !Array.isArray(rendered) ||
    rendered.length !== levels ||
    rendered.some((row) => !Array.isArray(row) || row.length !== variants.length)
  ) {
    throw new Error(
      `the bar renderer did not return ${String(levels)} x ${String(variants.length)} markups`,
    );
  }
  return rendered as readonly (readonly string[])[];
}

function harnessHtml(markup: string): string {
  const tokens = readFileSync(path.join(rendererDir, 'styles/tokens.css'), 'utf8');
  const base = readFileSync(path.join(rendererDir, 'styles/base.css'), 'utf8');
  const shelf = readFileSync(path.join(rendererDir, 'shelf/shelf.css'), 'utf8');
  return `<!doctype html><meta charset="utf-8"><style>
${fontFaces()}
${tokens}
${base}
${shelf}
html,body{margin:0;padding:0;overflow:hidden}
</style>${markup}`;
}

test('AC-P3-35-5 the top bar has a measured floor at the widest label, and nothing overlaps at it', async () => {
  const variants = sortVariants();
  expect(variants.length, 'a run that measured no sort variant proves nothing').toBeGreaterThan(0);
  // eslint-disable-next-line no-console -- the derivation is part of the measurement
  console.log(`measuring ${String(variants.length)} sort variant(s): ${variants.join(', ')}`);

  const perLevel = await barMarkupPerLevel(variants);
  // The markup really is the component's, and really does differ by level.
  expect(perLevel[0]?.[0]).toContain('CODOTHECA');
  expect(perLevel[3]?.[0]).not.toContain('CODOTHECA');

  const dir = mkdtempSync(path.join(tmpdir(), 'cdt-topbar-'));
  writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ name: 'h', main: 'main.cjs' }));
  writeFileSync(
    path.join(dir, 'main.cjs'),
    `const { app, BrowserWindow } = require('electron');
     app.whenReady().then(() => {
       new BrowserWindow({ width: 1600, height: 300, show: false, useContentSize: true })
         .loadURL('about:blank');
     });`,
  );

  const harness = await electron.launch({ args: [dir], cwd: appDir });
  const page = await harness.firstWindow();
  await page.waitForLoadState('domcontentloaded');

  /**
   * The bar is a block child of `body`, so setting the body's width sets the bar's exactly.
   *
   * Resizing the window instead lands within a pixel of the requested size on a fractionally
   * scaled display, and a bisection over a ±1px input answers ±1px — which is how the same run
   * reported two different shed widths for one build.
   */
  const setWidth = async (width: number): Promise<void> => {
    await page.evaluate((w) => {
      document.body.style.width = `${String(w)}px`;
    }, width);
  };

  const boxes = async (): Promise<
    { readonly slot: string; readonly left: number; readonly right: number }[]
  > =>
    page.$$eval('.cdt-shelf-bar > *', (nodes) =>
      nodes.map((node) => {
        const r = node.getBoundingClientRect();
        return {
          slot: node.getAttribute('data-slot') ?? node.getAttribute('class') ?? '',
          left: r.left,
          right: r.right,
        };
      }),
    );

  /** Both failure modes §8.0a describes: the row runs past the bar, so the right-hand control is
   *  clipped; or two boxes overlap and one paints over the other. */
  const failsAt = async (width: number): Promise<boolean> => {
    await setWidth(width);
    const overflows = await page.$eval(
      '.cdt-shelf-bar',
      (bar) => bar.scrollWidth > bar.clientWidth + 0.5,
    );
    if (overflows) return true;
    const measured = await boxes();
    return measured.slice(1).some((box, i) => box.left < (measured[i]?.right ?? 0) - 0.5);
  };

  /** Puts one shed level on screen at one sort variant, with its faces really loaded. Every
   *  measurement goes through this: a re-`setContent` re-parses the sheet, and measuring before
   *  the faces are ready measures a fallback bar. */
  const showLevel = async (level: number, variant = 0): Promise<void> => {
    await page.setContent(harnessHtml(perLevel[level]?.[variant] ?? ''));
    await page.evaluate(async () => {
      await document.fonts.ready;
    });
    // A measurement taken in a fallback face is a measurement of a different bar. The three
    // families are self-hosted, so if they did not load the numbers below mean nothing.
    const faces = await page.evaluate(() => ({
      loaded: document.fonts.size,
      // Weight 600 at 13px: the sort and density values, which every shed level renders. The
      // wordmark's 700 is gone by level 3, and `check` reports false for a face no node asked for.
      display: document.fonts.check('600 13px Rajdhani'),
      mono: document.fonts.check('400 10.5px "JetBrains Mono"'),
    }));
    // Only the two families the bar draws in: `check` reports false for a declared face nothing
    // on the page has asked for, and the bar carries no body copy.
    expect(faces, 'the bar was measured in fallback fonts').toEqual({
      loaded: 9,
      display: true,
      mono: true,
    });
  };

  /**
   * [p3] §35.6: the widest label is **measured**, not named and not counted in glyphs.
   *
   * Shed level 3 is where it matters — the `SORT` key is already gone and the value stands alone,
   * so the value's own box is the floor. Each variant is rendered in the real layout engine with
   * the real faces and the widest measured box wins.
   */
  const widestVariant = async (): Promise<{ index: number; width: number }> => {
    let best = { index: 0, width: -1 };
    for (let index = 0; index < variants.length; index += 1) {
      await showLevel(3, index);
      const width = await page.$eval(
        '[data-slot="sort"] .cdt-shelf-control-value',
        (node) => node.getBoundingClientRect().width,
      );
      const key = String(variants[index]);
      const label = SORT_LABELS[key as keyof typeof SORT_LABELS] ?? key;
      // eslint-disable-next-line no-console -- the measurement is the deliverable
      console.log(`sort value width: ${key} (${label}) = ${width.toFixed(2)}px`);
      if (width > best.width) best = { index, width };
    }
    return best;
  };

  const widest = await widestVariant();
  const widestKey = String(variants[widest.index]);
  // eslint-disable-next-line no-console -- the measurement is the deliverable
  console.log(
    `widest sort label: ${widestKey} (${SORT_LABELS[widestKey as keyof typeof SORT_LABELS] ?? widestKey}) at ${widest.width.toFixed(2)}px`,
  );

  /** The narrowest width at which this shed level still fits, at the widest label. Bisection,
   *  1px resolution. */
  const fitsFrom = async (level: number): Promise<number> => {
    await showLevel(level, widest.index);
    let bad = 400;
    let good = 1600;
    expect(await failsAt(good), `level ${String(level)} does not fit even at 1600px`).toBe(false);
    while (good - bad > 1) {
      const mid = Math.floor((good + bad) / 2);
      if (await failsAt(mid)) bad = mid;
      else good = mid;
    }
    return good;
  };

  const fits: readonly [number, number, number, number] = [
    await fitsFrom(0),
    await fitsFrom(1),
    await fitsFrom(2),
    await fitsFrom(3),
  ];

  // A step engages at and below the widest width the step above it could not fit.
  const measuredShedWidths = [fits[0] - 1, fits[1] - 1, fits[2] - 1];
  const measuredFloor = fits[3];

  // eslint-disable-next-line no-console -- the measurement is the deliverable
  console.log(
    `measured top-bar floor: ${String(measuredFloor)}px; shed widths: ${measuredShedWidths.join(', ')}`,
  );

  expect(TOP_BAR_FLOOR_PX).toBe(measuredFloor);
  expect([...SHED_WIDTHS]).toEqual(measuredShedWidths);

  // The floor is tight: one pixel narrower and the bar fails. Without this the bisection could
  // report any width at which the bar happens to fit and the constant would mean nothing.
  await showLevel(3, widest.index);
  expect(await failsAt(measuredFloor - 1)).toBe(true);
  expect(await failsAt(measuredFloor)).toBe(false);

  // And at the floor the bar still works: the query field is at its own declared 80px minimum
  // and no slot has been squeezed out of existence.
  await setWidth(measuredFloor);
  const fieldWidth = await page.$eval(
    '.cdt-shelf-field',
    (node) => node.getBoundingClientRect().width,
  );
  expect(fieldWidth).toBeGreaterThanOrEqual(80);
  expect((await boxes()).every((box) => box.right > box.left)).toBe(true);

  // At the floor the bar has fully shed, in the stated order.
  await showLevel(3, widest.index);
  await setWidth(TOP_BAR_FLOOR_PX);
  expect((await boxes()).map((b) => b.slot)).not.toContain('switch');
  expect(await page.textContent('.cdt-shelf-bar')).not.toContain('CODOTHECA');
  expect(await page.locator('.cdt-shelf-mark').count()).toBe(1);

  // The bar is 40px, plus its 1px rule.
  const height = await page.$eval('.cdt-shelf-bar', (n) => n.getBoundingClientRect().height);
  expect(height).toBeCloseTo(TOP_BAR_HEIGHT_PX + 1, 0);

  await harness.close();
});

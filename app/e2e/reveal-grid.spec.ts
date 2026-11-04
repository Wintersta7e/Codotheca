import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { expect, test } from '@playwright/test';

import { launchApp } from './launchApp.js';
import { build } from 'esbuild';

import type {
  BadgesListed,
  MotivationState,
  ProjectId,
  Reveal,
} from '../src/generated/protocol.js';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const rendererDir = path.join(appDir, 'src/renderer');
const nodeModules = path.resolve(appDir, '..', 'node_modules');

/**
 * §43.7: the reveal's grid floor follows its panel count — 3 + 3 at six (quiet), 4 + 3 at seven,
 * 4 + 4 at eight — in the 860px column. A track count is the layout engine's arithmetic over the
 * real stylesheet, so it is measured in the Chromium the app ships rather than asserted from the
 * CSS text: the stylesheet test pins which rule holds which floor, this pins what they lay out.
 *
 * The markup is the screen's own, rendered by `RevealScreen` in a child Node process (Playwright's
 * JSX transform cannot drive `renderToStaticMarkup` — `topbar-floor.spec.ts` explains), at tier
 * `off` so the entry cascade holds no panel mid-flight. Rows are read from `offsetTop`, which a
 * transform does not move.
 */
const basis = { projectsCovered: 4, projectsTotal: 4, historyComplete: true };

const reveal: Reveal = {
  spanDays: { value: 4_400, basis },
  projectCount: { value: 4, basis },
  languageCount: { value: 2, basis },
  bestYear: { value: 2021, basis },
  playtimeSeconds: { value: 0, basis },
  oldestStillAlive: { projectId: 1 as ProjectId, firstCommitAt: 1, basis },
};

const quiet: MotivationState = {
  shown: false,
  level: null,
  rings: null,
  xpBar: null,
  pendingLine: null,
  terms: null,
};

const shown: MotivationState = {
  ...quiet,
  shown: true,
  level: {
    state: 'computed',
    figures: { level: 6, xp: 935, levelAt: 800, nextLevelAt: 1_100, floored: false },
    pendingProjects: 0,
  },
  xpBar: 0.45,
  terms: [{ term: 'commitDay', xp: 935 }],
};

const noneDue: BadgesListed = { awards: [], due: null, basis };
const due: BadgesListed = {
  awards: [
    {
      kind: 'polyglot',
      earnedOn: '2021-11-04',
      projectId: null,
      evidence: {
        projectName: null,
        gapDays: null,
        year: 2021,
        languages: null,
        creditedSeconds: null,
        projects: null,
        runStartOn: null,
      },
      landed: true,
    },
  ],
  due: 'polyglot',
  basis,
};

/** The three counts §43.7 names, with the rows each must lay out. */
const CASES = [
  { panels: 6, rows: [3, 3], state: quiet, badges: noneDue },
  { panels: 7, rows: [4, 3], state: shown, badges: noneDue },
  { panels: 8, rows: [4, 4], state: shown, badges: due },
] as const;

/** §10.4a's reveal column: `920px` less its `30px` padding each side. */
const COLUMN_PX = 860;

async function renderReveals(): Promise<readonly string[]> {
  const dir = mkdtempSync(path.join(tmpdir(), 'cdt-reveal-render-'));
  const entry = path.join(dir, 'render.ts');
  const bundle = path.join(dir, 'render.cjs');
  writeFileSync(
    entry,
    `import { createElement } from 'react';
     import { renderToStaticMarkup } from 'react-dom/server';
     import { readFileSync } from 'node:fs';
     import { RevealScreen } from ${JSON.stringify(path.join(rendererDir, 'firstrun/RevealScreen.js'))};
     // The screen reads the window's focus on its first render, and a server render has no
     // window. Effects do not run here, so nothing is written either way.
     globalThis.document = { visibilityState: 'visible', hasFocus: () => false };
     const job = JSON.parse(readFileSync(process.argv[2], 'utf8'));
     const deps = { nowSecs: 1760000000, languageTally: [], referenceCount: 0, project: () => null };
     const write = () => Promise.resolve();
     process.stdout.write(JSON.stringify(job.cases.map((c) =>
       renderToStaticMarkup(createElement(RevealScreen, {
         reveal: job.reveal,
         deps,
         motivation: { state: c.state, badges: c.badges, acknowledge: write, markLanded: write },
         tier: 'off',
         onGoOn: () => undefined,
       })),
     )));`,
  );
  await build({
    entryPoints: [entry],
    outfile: bundle,
    bundle: true,
    platform: 'node',
    format: 'cjs',
    jsx: 'automatic',
    absWorkingDir: appDir,
    nodePaths: [nodeModules],
    // `?raw` and stylesheet imports have no meaning in a server render.
    loader: { '.css': 'empty' },
    plugins: [
      {
        name: 'ts-source-for-js-specifier',
        setup(b) {
          b.onResolve({ filter: /^\.{0,2}\/.*\.js$/ }, (args) => {
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
  const input = path.join(dir, 'job.json');
  writeFileSync(input, JSON.stringify({ reveal, cases: CASES }));
  const out = JSON.parse(
    execFileSync(process.execPath, [bundle, input], { cwd: appDir, encoding: 'utf8' }),
  ) as unknown;
  if (!Array.isArray(out) || out.length !== CASES.length) {
    throw new Error(`the reveal renderer did not return ${String(CASES.length)} markups`);
  }
  return out.map(String);
}

function harnessHtml(markup: string): string {
  const sheet = (rel: string): string => readFileSync(path.join(rendererDir, rel), 'utf8');
  return `<!doctype html><meta charset="utf-8"><style>
${sheet('styles/tokens.css')}
${sheet('styles/base.css')}
${sheet('firstrun/firstRun.css')}
html,body{margin:0;padding:0}
</style>${markup}`;
}

test('AC-P4-43-11-grid the reveal lays out 3 + 3 at six, 4 + 3 at seven and 4 + 4 at eight', async () => {
  const markups = await renderReveals();

  const dir = mkdtempSync(path.join(tmpdir(), 'cdt-reveal-grid-'));
  writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ name: 'h', main: 'main.cjs' }));
  writeFileSync(
    path.join(dir, 'main.cjs'),
    `const { app, BrowserWindow } = require('electron');
     app.commandLine.appendSwitch('force-device-scale-factor', '1');
     app.whenReady().then(() => {
       new BrowserWindow({ width: 1600, height: 1200, show: false, useContentSize: true })
         .loadURL('about:blank');
     });`,
  );
  const harness = await launchApp({ args: [dir], cwd: appDir });
  try {
    const page = await harness.firstWindow();
    await page.waitForLoadState('domcontentloaded');
    // Every count is measured and printed before any is asserted, so one failing count does
    // not hide what the others lay out.
    const all: { panels: number; width: number; rows: number[] }[] = [];
    for (const markup of markups) {
      await page.setContent(harnessHtml(markup));
      const measured = await page.evaluate(() => {
        const grid = document.querySelector('.cdt-fr-panels');
        const panels = [...document.querySelectorAll<HTMLElement>('[data-testid="fr-panel"]')];
        const tops = new Map<number, number>();
        for (const panel of panels) tops.set(panel.offsetTop, (tops.get(panel.offsetTop) ?? 0) + 1);
        return {
          width: grid === null ? 0 : grid.getBoundingClientRect().width,
          panels: panels.length,
          rows: [...tops.entries()].sort((a, b) => a[0] - b[0]).map(([, n]) => n),
        };
      });
      // eslint-disable-next-line no-console -- the measurement is the deliverable
      console.log(
        `${String(measured.panels)} panels in a ${measured.width.toFixed(1)}px column: rows ${measured.rows.join(' + ')}`,
      );
      all.push(measured);
    }
    expect(
      all.map((m) => m.panels),
      'a case that drew no panel measures nothing',
    ).toEqual(CASES.map((c) => c.panels));
    for (const m of all) expect(m.width).toBeCloseTo(COLUMN_PX, 0);
    expect(all.map((m) => m.rows)).toEqual(CASES.map((c) => c.rows));
  } finally {
    await harness.close();
  }
});

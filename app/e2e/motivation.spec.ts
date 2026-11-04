import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';
import { DatabaseSync } from 'node:sqlite';

import {
  _electron as electron,
  expect,
  test,
  type ElectronApplication,
  type Locator,
  type Page,
} from '@playwright/test';

import { FIXTURE_GIT_ENV, homeEnv } from './isolatedHome.js';

// Playwright transpiles specs to CommonJS, so `__dirname` is correct here and `import.meta` is
// not — the opposite of every vitest file in this repo.
const appDir = path.resolve(__dirname, '..');
const repoRoot = path.resolve(appDir, '..');

/**
 * §43's production path in the real app over a release core: every rise here is a real ledger
 * write the core made from back-dated commits a real scan read — never a hand-built payload
 * (§43.17). No XP price or level threshold is restated: a rise is driven by adding commit-days
 * until the band's own number moves, each iteration printed.
 */
const coreBinary = path.join(
  repoRoot,
  'core',
  'target',
  'release',
  process.platform === 'win32' ? 'codotheca-core.exe' : 'codotheca-core',
);
const NO_CORE = 'the release core binary is not built, so there is no core to rise through';

const EMAIL = 'e2e@example.invalid';
/** Enough repositories that one scan adds several commit-days. */
const REPOS = ['alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot'] as const;
/** The most scans one rise may take before the case fails rather than loops. */
const MAX_SCANS = 10;

/** `YYYY-MM-DD` in this machine's zone, which the core's clock reads too. */
function localDate(daysBack: number): string {
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
function seedHome(repos: readonly string[]): string {
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
function commitBackdated(home: string, repo: string, daysBack: number): void {
  const dir = path.join(home, 'src', repo);
  const date = `${localDate(daysBack)}T12:00:00`;
  writeFileSync(path.join(dir, `day-${String(daysBack)}.txt`), `${date}\n`, 'utf8');
  git(dir, ['add', '.']);
  git(dir, ['commit', '-qm', `day ${String(daysBack)}`], {
    GIT_AUTHOR_DATE: date,
    GIT_COMMITTER_DATE: date,
  });
}

function launch(home: string, userData: string): Promise<ElectronApplication> {
  return electron.launch({
    args: ['.', `--user-data-dir=${userData}`],
    cwd: appDir,
    env: { ...process.env, ...homeEnv(home), CODOTHECA_DATA_DIR: userData },
  });
}

async function openWindow(app: ElectronApplication): Promise<Page> {
  const window = await app.firstWindow();
  await window.waitForLoadState('domcontentloaded');
  return window;
}

/** §10 end to end: the roots screen, `DIG`, the reveal, the turn, the shelf. */
async function firstRun(window: Page, home: string, cards: number): Promise<void> {
  await window.waitForSelector('.cdt-fr-view--roots', { timeout: 60_000 });
  const corpusRow = window
    .locator('[data-testid="fr-root-row"][data-tickable="true"]')
    .filter({ hasText: home });
  await expect(corpusRow).toHaveAttribute('aria-checked', 'true');
  await window.getByRole('button', { name: 'DIG', exact: true }).click();
  const goOn = window.getByRole('button', { name: 'GO ON', exact: true });
  await goOn.waitFor({ state: 'visible', timeout: 150_000 });
  await goOn.click();
  const notNow = window.getByRole('button', { name: 'NOT NOW', exact: true });
  await notNow.waitFor({ state: 'visible', timeout: 60_000 });
  await notNow.click();
  await expect(window.locator('.cdt-card')).toHaveCount(cards, { timeout: 60_000 });
}

const band = (window: Page): Locator => window.locator('.cdt-band');
const lines = (window: Page): Locator => window.locator('.cdt-band-line');
const lineOf = (window: Page, form: string): Locator =>
  window.locator(`.cdt-band-line[data-form="${form}"]`);
/** The digit that is always mounted; the leaving one exists only while a flip plays. */
const digit = (window: Page): Locator => window.locator('.cdt-level-digit:not([data-flip="old"])');

/** What the band draws now, for the printed trace: its level and the XP line's figure. */
async function bandReading(window: Page): Promise<string> {
  const level = await window.locator('.cdt-band-level-number').getAttribute('aria-label');
  const xp = (await window.locator('.cdt-band-xp-line').textContent()) ?? '';
  return `${level ?? '?'} · xp line ${xp.slice(xp.lastIndexOf('›') + 1).trim()}`;
}

/** Animations running on the level digits, by name. */
function digitAnimations(window: Page): Promise<string[]> {
  return window.evaluate(() =>
    [...document.querySelectorAll('.cdt-level-digit')].flatMap((el) =>
      el.getAnimations().map((a) => (a instanceof CSSAnimation ? a.animationName : 'other')),
    ),
  );
}

/** Clicks a line, which opens Profile, and comes back to the shelf. */
async function clickLine(window: Page, form: string): Promise<void> {
  await lineOf(window, form).click();
  await expect(window.locator('.cdt-profile')).toBeVisible({ timeout: 30_000 });
  await window.locator('.cdt-profile-back').click();
  await expect(band(window)).toBeVisible({ timeout: 30_000 });
}

/**
 * Back-dated commit-days, one new date per repository per scan, until the band's own number rises
 * and a `rise` line renders. Each scan's reading is printed; nothing here knows what a commit-day
 * is worth or where a level starts.
 */
async function riseThroughScans(window: Page, home: string, nextDay: { n: number }): Promise<void> {
  const before = await bandReading(window);
  // eslint-disable-next-line no-console -- the trace is the deliverable
  console.log(`rise: start at ${before}`);
  for (let scan = 1; scan <= MAX_SCANS; scan += 1) {
    const reading = await bandReading(window);
    for (const repo of REPOS) {
      commitBackdated(home, repo, nextDay.n);
      nextDay.n += 1;
    }
    await window.locator('[data-slot="scan"]').click();
    await expect
      .poll(
        async () =>
          (await lineOf(window, 'rise').count()) > 0 || (await bandReading(window)) !== reading,
        { timeout: 90_000, message: `scan ${String(scan)} moved nothing the band draws` },
      )
      .toBe(true);
    // The XP line can move a beat before the line arrives in the same publication.
    await window.waitForTimeout(500);
    const risen = (await lineOf(window, 'rise').count()) > 0;
    // eslint-disable-next-line no-console -- the trace is the deliverable
    console.log(
      `rise: scan ${String(scan)} → ${await bandReading(window)}; rise line ${String(risen)}`,
    );
    if (risen) return;
  }
  throw new Error(`no rise line after ${String(MAX_SCANS)} scans`);
}

/** The index's own file, where the core keeps it under the data directory. */
const indexPath = (userData: string): string => path.join(userData, 'index.db');

/**
 * Fixture state, written while no core runs: day one eight days back, so §39's rings are drawn
 * rather than waived. Day one is today on any new install and no clock can be moved under a
 * release core; the rings' colours and transitions are what the cases below measure.
 */
function backdateDayOne(userData: string): void {
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

test.describe.serial('the band over a real ledger', () => {
  test.skip(!existsSync(coreBinary), NO_CORE);

  let home = '';
  let userData = '';
  let app: ElectronApplication | null = null;
  let window: Page;
  /** The next back-dated day a commit takes; every commit-day this file adds is a new date. */
  const nextDay = { n: 9 };

  const relaunch = async (): Promise<void> => {
    await app?.close();
    app = await launch(home, userData);
    window = await openWindow(app);
    await expect(band(window)).toBeVisible({ timeout: 60_000 });
  };

  test.beforeAll(async () => {
    test.setTimeout(300_000);
    home = seedHome(REPOS);
    userData = mkdtempSync(path.join(tmpdir(), 'codotheca-motivation-'));
    app = await launch(home, userData);
    window = await openWindow(app);
    await firstRun(window, home, REPOS.length);
    await expect(band(window)).toBeVisible({ timeout: 60_000 });
    await app.close();
    app = null;
    backdateDayOne(userData);
    await relaunch();
    // Whatever first run left waiting — an arrival over a level the reveal did not show, a badge
    // — is clicked away, so the line each case reads is the one its own rise made.
    for (let n = 0; n < 5 && (await lines(window).count()) > 0; n += 1) {
      const form = (await lines(window).first().getAttribute('data-form')) ?? '';
      // eslint-disable-next-line no-console -- the trace is the deliverable
      console.log(`setup: first run left a ${form} line; clicked`);
      await clickLine(window, form);
    }
    await expect(lines(window)).toHaveCount(0);
  });

  test.afterAll(async () => {
    await app?.close();
  });

  test('AC-P4-43-3 a real rise renders one line that survives a page, a reload and a restart, and clears only on its click', async () => {
    test.setTimeout(600_000);
    await riseThroughScans(window, home, nextDay);
    await expect(lines(window)).toHaveCount(1);
    await expect(lineOf(window, 'rise')).toHaveCount(1);

    // No interruption: nothing in the document is a dialog or an alert, and nothing fixed
    // covers the line.
    const overlay = await window.evaluate(() => {
      const line = document.querySelector('.cdt-band-line');
      if (line === null) return { roles: -1, covering: ['no line'], hit: false };
      const roles = document.querySelectorAll(
        '[role="dialog"],[role="alertdialog"],[role="alert"]',
      ).length;
      const covering = [...document.querySelectorAll<HTMLElement>('body *')]
        .filter((el) => getComputedStyle(el).position === 'fixed' && !el.contains(line))
        .filter((el) => {
          const r = el.getBoundingClientRect();
          return r.width >= innerWidth * 0.9 && r.height >= innerHeight * 0.9;
        })
        .map((el) => el.className);
      const box = line.getBoundingClientRect();
      const top = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
      return { roles, covering, hit: top !== null && line.contains(top) };
    });
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(`AC-P4-43-3 overlay: ${JSON.stringify(overlay)}`);
    expect(overlay).toEqual({ roles: 0, covering: [], hit: true });

    // A project page and back.
    await window
      .locator('.cdt-card')
      .first()
      .click({ modifiers: ['Shift'] });
    await window.waitForSelector('[data-testid="cp-page"]', { timeout: 60_000 });
    await window.locator('.cp-back').first().click();
    await expect(band(window)).toBeVisible({ timeout: 30_000 });
    await expect(lineOf(window, 'rise')).toHaveCount(1);

    // A window reload.
    await window.reload();
    await expect(band(window)).toBeVisible({ timeout: 60_000 });
    await expect(lineOf(window, 'rise')).toHaveCount(1);

    // An app restart over the same data directory.
    await relaunch();
    await expect(lineOf(window, 'rise')).toHaveCount(1);
    // eslint-disable-next-line no-console -- the trace is the deliverable
    console.log('AC-P4-43-3: the rise line survived a page, a reload and a restart');

    // Its click — and only its click — clears it, opening Profile on the way.
    await lineOf(window, 'rise').click();
    await expect(window.locator('.cdt-profile')).toBeVisible({ timeout: 30_000 });
    await window.locator('.cdt-profile-back').click();
    await expect(band(window)).toBeVisible({ timeout: 30_000 });
    await expect(lineOf(window, 'rise')).toHaveCount(0);
  });

  test('AC-P4-43-8-app a real rise flips on the next keydown, and a reload paints none', async () => {
    test.setTimeout(600_000);
    await riseThroughScans(window, home, nextDay);
    const armed = await digitAnimations(window);
    await window.keyboard.press('Shift');
    const fired = await digitAnimations(window);
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(
      `AC-P4-43-8-app: before the key ${JSON.stringify(armed)}, after ${JSON.stringify(fired)}`,
    );
    expect(armed).toEqual([]);
    expect(fired.length).toBeGreaterThan(0);

    await window.reload();
    await digit(window).waitFor({ state: 'visible', timeout: 60_000 });
    const samples: number[] = [];
    for (let i = 0; i < 6; i += 1) {
      samples.push((await digitAnimations(window)).length);
      await window.waitForTimeout(250);
    }
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(`AC-P4-43-8-app: digit animations after the reload ${JSON.stringify(samples)}`);
    expect(samples.every((n) => n === 0)).toBe(true);
    // The line the rise made is still pending, for the cases that read the band at rest.
    await expect(lineOf(window, 'rise')).toHaveCount(1);
  });

  /** Sets §11.6's tier through the drawer and returns the tier the document resolved. */
  const setTier = async (label: 'AUTO' | 'FULL' | 'REDUCED' | 'OFF'): Promise<string> => {
    await window.getByRole('button', { name: 'Settings', exact: true }).click();
    await window.getByRole('radio', { name: label, exact: true }).click();
    await expect(window.getByRole('radio', { name: label, exact: true })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    await window.keyboard.press('Escape');
    await expect(window.getByRole('dialog')).toHaveCount(0);
    return (await window.evaluate(() => document.documentElement.dataset['effectsTier'])) ?? '';
  };

  test('AC-P4-43-10 at rest the band runs no animation, and a ring, bar or cell changes by a cut at every tier', async () => {
    test.setTimeout(120_000);
    await expect(band(window)).toBeVisible();
    await expect(lines(window)).toHaveCount(1);
    await expect(window.locator('.cdt-band-ring-glyph').first()).toBeVisible();

    // No input for five seconds, then ten seconds of samples.
    await window.waitForTimeout(5_000);
    const counts: number[] = [];
    for (let i = 0; i < 20; i += 1) {
      counts.push(await window.evaluate(() => document.getAnimations().length));
      await window.waitForTimeout(500);
    }
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(`AC-P4-43-10: document.getAnimations() over 10 s ${JSON.stringify(counts)}`);
    expect(counts.every((n) => n === 0)).toBe(true);

    for (const label of ['AUTO', 'FULL', 'REDUCED', 'OFF'] as const) {
      const resolved = await setTier(label);
      const durations = await window.evaluate(() =>
        ['.cdt-band-ring-glyph', '.cdt-band-xp-fill', '.cdt-band-cell'].map((selector) => {
          const el = document.querySelector(selector);
          return el === null ? 'absent' : getComputedStyle(el).transitionDuration;
        }),
      );
      // eslint-disable-next-line no-console -- the measurement is the deliverable
      console.log(
        `AC-P4-43-10 ${label} (${resolved}): ring, bar, cell ${JSON.stringify(durations)}`,
      );
      expect(durations).toEqual(['0s', '0s', '0s']);
    }
    await setTier('AUTO');
  });

  test('AC-P4-43-20 every band text meets 5.9:1 on its ground, and the bar, arcs and cells 3:1', async () => {
    test.setTimeout(120_000);
    await expect(band(window)).toBeVisible();
    const measured = await window.evaluate(() => {
      const canvas = document.createElement('canvas');
      canvas.width = 1;
      canvas.height = 1;
      const ctx = canvas.getContext('2d', { willReadFrequently: true });
      if (ctx === null) throw new Error('no 2d context to composite colours in');
      /** Paints `layers` bottom to top on one pixel and reads what the pixel holds. */
      const paint = (layers: readonly string[]): number[] => {
        ctx.clearRect(0, 0, 1, 1);
        for (const layer of layers) {
          ctx.fillStyle = layer;
          ctx.fillRect(0, 0, 1, 1);
        }
        return [...ctx.getImageData(0, 0, 1, 1).data.slice(0, 4)];
      };
      /** The background colours under `el`, outermost first, from the first opaque one. */
      const ground = (el: Element | null): string[] => {
        const layers: string[] = [];
        for (let node = el; node !== null; node = node.parentElement) {
          const colour = getComputedStyle(node).backgroundColor;
          layers.unshift(colour);
          if (paint([colour])[3] === 255) return layers;
        }
        throw new Error('no opaque ground under the band');
      };
      const channel = (v: number): number => {
        const c = v / 255;
        return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
      };
      const luminance = (rgb: readonly number[]): number =>
        0.2126 * channel(rgb[0] ?? 0) +
        0.7152 * channel(rgb[1] ?? 0) +
        0.0722 * channel(rgb[2] ?? 0);
      const ratio = (a: readonly number[], b: readonly number[]): number => {
        const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
        return ((hi ?? 0) + 0.05) / ((lo ?? 0) + 0.05);
      };
      const hex = (rgb: readonly number[]): string =>
        `#${rgb
          .slice(0, 3)
          .map((v) => v.toString(16).padStart(2, '0'))
          .join('')}`;

      const root = document.querySelector('.cdt-band');
      if (root === null) throw new Error('no band');
      const texts: { what: string; fg: string; bg: string; ratio: number }[] = [];
      for (const el of root.querySelectorAll<HTMLElement>('*')) {
        const own = [...el.childNodes].some(
          (n) => n.nodeType === Node.TEXT_NODE && (n.textContent ?? '').trim() !== '',
        );
        if (!own || el.getBoundingClientRect().width === 0) continue;
        const layers = ground(el.parentElement);
        // A ring's centre sits on the glyph's inner disc, which a pseudo-element paints.
        const glyph = el.closest('.cdt-band-ring-glyph');
        if (glyph !== null) layers.push(getComputedStyle(glyph, '::before').backgroundColor);
        const bg = paint(layers);
        const fg = paint([...layers, getComputedStyle(el).color]);
        texts.push({ what: el.className, fg: hex(fg), bg: hex(bg), ratio: ratio(fg, bg) });
      }

      const pairs: { what: string; a: string; b: string; ratio: number }[] = [];
      const pair = (what: string, a: number[], b: number[]): void => {
        pairs.push({ what, a: hex(a), b: hex(b), ratio: ratio(a, b) });
      };

      // The bar: its fill against the track it fills.
      const fill = root.querySelector('.cdt-band-xp-fill');
      const track = root.querySelector('.cdt-band-xp-track');
      if (fill === null || track === null) throw new Error('no XP bar drawn');
      const underTrack = [...ground(track.parentElement), getComputedStyle(track).backgroundColor];
      pair(
        'bar fill / track',
        paint([...underTrack, getComputedStyle(fill).backgroundColor]),
        paint(underTrack),
      );

      // The arc and the track, the two stops of the glyph's gradient as the stylesheet resolves
      // them. A ring with nothing to draw today carries the track alone, so the arc's state is
      // set on one glyph for the reading and put back.
      const glyph = root.querySelector<HTMLElement>('.cdt-band-ring-glyph');
      if (glyph === null) throw new Error('no ring drawn');
      const wasFill = glyph.dataset['fill'];
      glyph.dataset['fill'] = 'arc';
      const gradient = getComputedStyle(glyph).backgroundImage;
      if (wasFill === undefined) delete glyph.dataset['fill'];
      else glyph.dataset['fill'] = wasFill;
      /** `text` split on `separator` outside any parentheses. */
      const split = (text: string, separator: string): string[] => {
        const parts = [''];
        let depth = 0;
        for (const ch of text) {
          if (ch === '(') depth += 1;
          if (ch === ')') depth -= 1;
          if (depth === 0 && ch === separator) parts.push('');
          else parts.push(`${parts.pop() ?? ''}${ch}`);
        }
        return parts.map((p) => p.trim()).filter((p) => p !== '');
      };
      // Each stop is a colour then its positions; the colour is the longest leading run of
      // words the browser parses as one. A two-position stop resolves as two stops of one
      // colour, so the arc is the first distinct colour and the track the last.
      const stops = [
        ...new Set(
          split(gradient.slice(gradient.indexOf('(') + 1, gradient.lastIndexOf(')')), ',')
            .map((stop) => {
              const words = split(stop, ' ');
              while (words.length > 0 && !CSS.supports('color', words.join(' '))) words.pop();
              return words.join(' ');
            })
            .filter((colour) => colour !== ''),
        ),
      ];
      const [arc, rest] = [stops[0], stops[stops.length - 1]];
      if (stops.length !== 2 || arc === undefined || rest === undefined) {
        throw new Error(`the arc gradient did not resolve to an arc and a track: ${gradient}`);
      }
      const underGlyph = ground(glyph.parentElement);
      pair('ring arc / track', paint([...underGlyph, arc]), paint([...underGlyph, rest]));

      // An active day against an inactive one, both read from the stylesheet on one real cell.
      const cell = root.querySelector<HTMLElement>('.cdt-band-cell');
      if (cell === null) throw new Error('no rhythm cell drawn');
      const wasDay = cell.dataset['day'];
      const underCell = ground(cell.parentElement);
      cell.dataset['day'] = 'active';
      const active = paint([...underCell, getComputedStyle(cell).backgroundColor]);
      cell.dataset['day'] = 'inactive';
      const inactive = paint([...underCell, getComputedStyle(cell).backgroundColor]);
      if (wasDay === undefined) delete cell.dataset['day'];
      else cell.dataset['day'] = wasDay;
      pair('active cell / inactive cell', active, inactive);

      return { texts, pairs };
    });
    for (const t of measured.texts) {
      // eslint-disable-next-line no-console -- the measurement is the deliverable
      console.log(`AC-P4-43-20 text ${t.what}: ${t.fg} on ${t.bg} = ${t.ratio.toFixed(2)}:1`);
    }
    for (const p of measured.pairs) {
      // eslint-disable-next-line no-console -- the measurement is the deliverable
      console.log(`AC-P4-43-20 ${p.what}: ${p.a} against ${p.b} = ${p.ratio.toFixed(2)}:1`);
    }
    expect(measured.texts.length, 'a band with no text measured nothing').toBeGreaterThan(0);
    expect(measured.texts.filter((t) => t.ratio < 5.9)).toEqual([]);
    expect(measured.pairs).toHaveLength(3);
    expect(measured.pairs.filter((p) => p.ratio < 3)).toEqual([]);
  });

  test('AC-P4-43-9-app at reduced and at off the flip resolves no animation in the real renderer', async () => {
    test.setTimeout(120_000);
    /** The always-mounted digit given the flip's playing state, and the animation it resolves. */
    const flipped = (): Promise<string> =>
      window.evaluate(() => {
        const el = document.querySelector<HTMLElement>('.cdt-level-digit:not([data-flip])');
        if (el === null) return 'no resting digit';
        el.dataset['flip'] = 'new';
        const name = getComputedStyle(el).animationName;
        delete el.dataset['flip'];
        return name;
      });

    // The stylesheet is loaded: at full, the same element resolves the flip's own animation.
    const full = await setTier('FULL');
    const atFull = await flipped();
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(`AC-P4-43-9-app FULL (${full}): ${atFull}`);
    expect(full).toBe('full');
    expect(atFull).not.toBe('none');
    expect(atFull).not.toBe('no resting digit');

    for (const [label, tier] of [
      ['REDUCED', 'reduced'],
      ['OFF', 'off'],
    ] as const) {
      const resolved = await setTier(label);
      const name = await flipped();
      // eslint-disable-next-line no-console -- the measurement is the deliverable
      console.log(`AC-P4-43-9-app ${label} (${resolved}): ${name}`);
      expect(resolved).toBe(tier);
      expect(name).toBe('none');
    }
    await setTier('AUTO');
  });
});

test('AC-P4-43-13-app a new install quit while scanning relaunches to the shelf and an arrival line', async () => {
  test.skip(!existsSync(coreBinary), NO_CORE);
  test.setTimeout(400_000);
  const repos = ['one', 'two', 'three'];
  const home = seedHome(repos);
  const userData = mkdtempSync(path.join(tmpdir(), 'codotheca-arrival-'));

  const first = await launch(home, userData);
  try {
    const window = await openWindow(first);
    await window.waitForSelector('.cdt-fr-view--roots', { timeout: 60_000 });
    await window.getByRole('button', { name: 'DIG', exact: true }).click();
    // Read on every frame, so the quit lands on the scanning screen the moment it counts one.
    const at = await window.waitForFunction(
      () => {
        if (document.querySelector('.cdt-fr-view--reveal') !== null) return 'the reveal';
        const count = document.querySelector('[data-testid="fr-count-big"]')?.textContent;
        return count === undefined ? false : `${count} found`;
      },
      null,
      { polling: 'raf', timeout: 60_000 },
    );
    const where = String(await at.jsonValue());
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(`AC-P4-43-13-app: quit on the scanning screen at ${where}`);
    expect(where, 'the scan finished before it could be quit').not.toBe('the reveal');
  } finally {
    await first.close();
  }

  const second = await launch(home, userData);
  try {
    const window = await openWindow(second);
    await window.waitForSelector('.cdt-shelf, .cdt-fr-view', { timeout: 60_000 });
    await expect(window.locator('.cdt-shelf')).toBeVisible();
    await expect(band(window)).toBeVisible({ timeout: 60_000 });
    // The walk wrote rows and was cut off before their history was read, so the level is not
    // computed yet; the shelf's own SCAN is the first run to end in this lifetime (§43.10's
    // second path), and the read it finishes makes the level computable.
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(
      `AC-P4-43-13-app: relaunched to the shelf, ${String(await window.locator('.cdt-card').count())} cards, ` +
        `${(await window.locator('.cdt-band-level-number').getAttribute('aria-label')) ?? '?'}, ` +
        `${String(await lines(window).count())} line`,
    );
    await window.locator('[data-slot="scan"]').click();
    // No reveal replays while the arrival is awaited: every sample is taken on the shelf.
    let samples = 0;
    while ((await lineOf(window, 'arrival').count()) === 0) {
      expect(await window.locator('.cdt-fr-view').count(), 'first run replayed').toBe(0);
      expect(samples, 'no arrival line within two minutes').toBeLessThan(480);
      samples += 1;
      await window.waitForTimeout(250);
    }
    await expect(window.locator('.cdt-shelf')).toBeVisible();
    await expect(window.locator('.cdt-fr-view')).toHaveCount(0);
    const level = await digit(window).textContent();
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(
      `AC-P4-43-13-app: arrival after ${String(samples)} samples at level ${level ?? '?'}`,
    );

    // An arrival arms no flip and counts nothing up: input then plays nothing, and the number
    // holds still.
    await window.keyboard.press('Shift');
    const held: { level: string | null; animations: number }[] = [];
    for (let i = 0; i < 6; i += 1) {
      held.push({
        level: await digit(window).textContent(),
        animations: (await digitAnimations(window)).length,
      });
      await window.waitForTimeout(250);
    }
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(`AC-P4-43-13-app: after a key ${JSON.stringify(held)}`);
    expect(held.every((h) => h.animations === 0 && h.level === level)).toBe(true);

    await lineOf(window, 'arrival').click();
    await expect(window.locator('.cdt-profile')).toBeVisible({ timeout: 30_000 });
    await window.locator('.cdt-profile-back').click();
    await expect(band(window)).toBeVisible({ timeout: 30_000 });
    await expect(lineOf(window, 'arrival')).toHaveCount(0);
  } finally {
    await second.close();
  }
});

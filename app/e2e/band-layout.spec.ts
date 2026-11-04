import { existsSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import * as path from 'node:path';

import { expect, test, type ElectronApplication, type Page } from '@playwright/test';

import {
  backdateDayOne,
  commitBackdated,
  coreBinary,
  launch,
  NO_CORE,
  openWindow,
  seedHome,
} from './ledgerFixture.js';

/**
 * §43.2's band, §43.13's Profile and §43.7's reveal laid out by the Chromium the app ships, in a
 * real window over a real ledger, with the interim copy as it ships. jsdom has no layout, so every
 * box here is `getBoundingClientRect` in the running product, each width's figures printed.
 *
 * The band is measured at 1440, at each top-bar shed width and one above it, and at the 526 floor:
 * nothing it holds may leave its content box, no divider may open or close a wrapped row, and no
 * ring's centre may paint outside its ring.
 */

const REPOS = ['alpha', 'bravo', 'charlie', 'delta', 'echo', 'foxtrot'] as const;
/** 1440, each shed width (§36.2 rule 8) with the width one above it, and the 526 floor. */
const BAND_WIDTHS = [1440, 1156, 1155, 879, 878, 734, 733, 647, 646, 526] as const;
const HEIGHT = 873;
/** Optional: a directory the run writes its band screenshots into, for a person to look at. */
const SHOTS = process.env['BAND_LAYOUT_SHOTS'];

async function setContentWidth(app: ElectronApplication, page: Page, width: number): Promise<void> {
  await app.evaluate(
    ({ BrowserWindow }, size) => {
      BrowserWindow.getAllWindows()[0]?.setContentSize(size.width, size.height);
    },
    { width, height: HEIGHT },
  );
  await page.waitForFunction((w) => window.innerWidth === w, width);
  // Two frames: one for the layout, one for anything a resize observer then settles.
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            resolve();
          });
        });
      }),
  );
}

interface BandMeasure {
  readonly clientWidth: number;
  readonly scrollWidth: number;
  readonly measured: number;
  readonly escapes: string[];
  readonly rows: string[];
  readonly stranded: string[];
  readonly dividers: number;
  readonly centres: { readonly text: number; readonly box: string; readonly contained: boolean }[];
}

/** Every band box against the band's content box, its rows in order, and each ring centre. */
function measureBand(window: Page): Promise<BandMeasure | null> {
  return window.evaluate(() => {
    const band = document.querySelector<HTMLElement>('.cdt-band');
    if (band === null) return null;
    const px = (value: string): number => Number.parseFloat(value) || 0;
    const style = getComputedStyle(band);
    const outer = band.getBoundingClientRect();
    const box = {
      left: outer.left + px(style.borderLeftWidth) + px(style.paddingLeft),
      right: outer.right - px(style.borderRightWidth) - px(style.paddingRight),
      top: outer.top + px(style.borderTopWidth) + px(style.paddingTop),
      bottom: outer.bottom - px(style.borderBottomWidth) - px(style.paddingBottom),
    };
    const name = (el: Element): string =>
      `${el.tagName.toLowerCase()}.${[...el.classList].join('.')}`;
    const span = (r: DOMRect): string =>
      `x${r.left.toFixed(1)}–${r.right.toFixed(1)} y${r.top.toFixed(1)}–${r.bottom.toFixed(1)}`;
    const shown = (el: Element): boolean => {
      const s = getComputedStyle(el);
      const r = el.getBoundingClientRect();
      return s.display !== 'none' && s.visibility !== 'hidden' && r.width > 0 && r.height > 0;
    };

    const escapes: string[] = [];
    let measured = 0;
    for (const el of band.querySelectorAll('*')) {
      if (!shown(el)) continue;
      measured += 1;
      const r = el.getBoundingClientRect();
      const out =
        r.left < box.left - 0.5 ||
        r.right > box.right + 0.5 ||
        r.top < box.top - 0.5 ||
        r.bottom > box.bottom + 0.5;
      if (out) escapes.push(`${name(el)} ${span(r)}`);
    }

    // Rows in DOM order: an item starts a new row once it sits wholly below the current one.
    const items = [...band.children].filter(shown);
    const rows: Element[][] = [];
    let bottom = -Infinity;
    for (const el of items) {
      const r = el.getBoundingClientRect();
      const current = rows.at(-1);
      if (current === undefined || r.top >= bottom - 0.5) {
        rows.push([el]);
        bottom = r.bottom;
      } else {
        current.push(el);
        bottom = Math.max(bottom, r.bottom);
      }
    }
    const stranded: string[] = [];
    rows.forEach((row, index) => {
      const first = row[0];
      const last = row.at(-1);
      if (first?.classList.contains('cdt-band-divider') === true) {
        stranded.push(
          `row ${String(index + 1)} opens on a divider ${span(first.getBoundingClientRect())}`,
        );
      }
      if (last !== first && last?.classList.contains('cdt-band-divider') === true) {
        stranded.push(
          `row ${String(index + 1)} closes on a divider ${span(last.getBoundingClientRect())}`,
        );
      }
    });

    const centres = [...band.querySelectorAll<HTMLElement>('.cdt-band-ring-centre')].map((el) => {
      const r = el.getBoundingClientRect();
      const ring = el.closest('.cdt-band-ring-glyph')?.getBoundingClientRect() ?? null;
      const s = getComputedStyle(el);
      const clips = (v: string): boolean => v === 'hidden' || v === 'clip';
      const fits = el.scrollWidth <= el.clientWidth && el.scrollHeight <= el.clientHeight;
      const inside =
        ring !== null &&
        r.left >= ring.left - 0.5 &&
        r.right <= ring.right + 0.5 &&
        r.top >= ring.top - 0.5 &&
        r.bottom <= ring.bottom + 0.5;
      return {
        text: el.textContent.length,
        box: `${span(r)} scroll ${String(el.scrollWidth)}×${String(el.scrollHeight)} overflow ${s.overflowX}/${s.overflowY}`,
        contained: inside && (fits || (clips(s.overflowX) && clips(s.overflowY))),
      };
    });

    return {
      clientWidth: band.clientWidth,
      scrollWidth: band.scrollWidth,
      measured,
      escapes,
      rows: rows.map(
        (row) =>
          `[${row.map((el) => `${[...el.classList].join('.')} ${el.getBoundingClientRect().left.toFixed(0)}`).join(' | ')}]`,
      ),
      stranded,
      dividers: band.querySelectorAll('.cdt-band-divider').length,
      centres,
    };
  });
}

/** Measures the band at every width and returns what failed; every width's figures are printed. */
async function bandAtEveryWidth(
  app: ElectronApplication,
  window: Page,
  label: string,
  centresExpected: boolean,
): Promise<string[]> {
  const failures: string[] = [];
  for (const width of BAND_WIDTHS) {
    await setContentWidth(app, window, width);
    const m = await measureBand(window);
    if (m === null) {
      failures.push(`${label} ${String(width)}: no band`);
      continue;
    }
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(
      `${label} ${String(width)}: band client ${String(m.clientWidth)} scroll ${String(m.scrollWidth)}, ` +
        `${String(m.measured)} boxes, ${String(m.dividers)} divider(s), rows ${m.rows.join(' ')}` +
        (m.escapes.length === 0 ? '' : `\n  escapes: ${m.escapes.join('; ')}`) +
        (m.stranded.length === 0 ? '' : `\n  stranded: ${m.stranded.join('; ')}`) +
        m.centres
          .map(
            (c) => `\n  centre (${String(c.text)} chars) ${c.box} contained ${String(c.contained)}`,
          )
          .join(''),
    );
    if (SHOTS !== undefined && (width === 1440 || width === 879)) {
      await window.screenshot({ path: path.join(SHOTS, `band-${label}-${String(width)}.png`) });
    }
    const at = `${label} ${String(width)}`;
    if (m.measured === 0) failures.push(`${at}: measured no box`);
    if (m.dividers === 0) failures.push(`${at}: no divider to measure`);
    if (m.scrollWidth > m.clientWidth) {
      failures.push(
        `${at}: scrollWidth ${String(m.scrollWidth)} > clientWidth ${String(m.clientWidth)}`,
      );
    }
    for (const escape of m.escapes) failures.push(`${at}: outside the band: ${escape}`);
    for (const strand of m.stranded) failures.push(`${at}: ${strand}`);
    if (centresExpected && m.centres.length === 0) failures.push(`${at}: no ring centre measured`);
    for (const centre of m.centres) {
      if (!centre.contained)
        failures.push(`${at}: a ring centre paints outside its ring: ${centre.box}`);
    }
  }
  return failures;
}

/** Waits out the reveal's entry cascade, so no panel is measured mid-flight. */
async function revealSettled(window: Page): Promise<void> {
  await window.waitForFunction(() =>
    [...document.querySelectorAll('.cdt-fr-panel')].every((panel) =>
      panel.getAnimations({ subtree: true }).every((a) => a.playState === 'finished'),
    ),
  );
}

interface RevealMeasure {
  readonly panels: number;
  readonly escapes: string[];
  readonly heads: string[];
  readonly gaps: string[];
}

function measureReveal(window: Page): Promise<RevealMeasure> {
  return window.evaluate(() => {
    const px = (value: string): number => Number.parseFloat(value) || 0;
    const panels = [...document.querySelectorAll<HTMLElement>('.cdt-fr-panel')];
    const escapes: string[] = [];
    const gaps: string[] = [];
    const rows = new Map<number, number[]>();
    for (const panel of panels) {
      const s = getComputedStyle(panel);
      const p = panel.getBoundingClientRect();
      const inner = {
        left: p.left + px(s.borderLeftWidth),
        right: p.right - px(s.borderRightWidth),
        top: p.top + px(s.borderTopWidth),
        bottom: p.bottom - px(s.borderBottomWidth),
      };
      const key = panel.querySelector('.cdt-fr-panel-key')?.textContent ?? '?';
      for (const el of panel.querySelectorAll('*')) {
        const r = el.getBoundingClientRect();
        if (r.width === 0 && r.height === 0) continue;
        if (
          r.left < inner.left - 0.5 ||
          r.right > inner.right + 0.5 ||
          r.top < inner.top - 0.5 ||
          r.bottom > inner.bottom + 0.5
        ) {
          escapes.push(
            `${key}: ${[...el.classList].join('.')} x${r.left.toFixed(1)}–${r.right.toFixed(1)} past x${inner.left.toFixed(1)}–${inner.right.toFixed(1)}`,
          );
        }
      }
      const head = panel.querySelector('.cdt-fr-panel-head')?.getBoundingClientRect();
      const row = Math.round(p.top);
      rows.set(row, [...(rows.get(row) ?? []), (head?.top ?? Number.NaN) - p.top]);
      const value = panel.querySelector('.cdt-fr-panel-value')?.getBoundingClientRect();
      const caption = panel.querySelector('.cdt-fr-panel-caption')?.getBoundingClientRect();
      if (value !== undefined && caption !== undefined) {
        gaps.push(`${key}: ${(caption.top - value.bottom).toFixed(1)}`);
      }
    }
    const heads = [...rows.entries()].map(
      ([top, offsets]) => `row y${String(top)}: ${offsets.map((o) => o.toFixed(1)).join('/')}`,
    );
    return { panels: panels.length, escapes, heads, gaps };
  });
}

async function revealFailures(app: ElectronApplication, window: Page): Promise<string[]> {
  const failures: string[] = [];
  for (const width of [1440, 879]) {
    await setContentWidth(app, window, width);
    await revealSettled(window);
    const m = await measureReveal(window);
    if (SHOTS !== undefined) {
      await window.screenshot({ path: path.join(SHOTS, `reveal-${String(width)}.png`) });
    }
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(
      `reveal ${String(width)}: ${String(m.panels)} panels; head offsets ${m.heads.join('; ')}; ` +
        `caption below value by ${m.gaps.join(', ')}` +
        (m.escapes.length === 0 ? '' : `\n  escapes: ${m.escapes.join('; ')}`),
    );
    const at = `reveal ${String(width)}`;
    if (m.panels < 6) failures.push(`${at}: ${String(m.panels)} panels measured`);
    if (m.gaps.length === 0) failures.push(`${at}: no value and caption measured`);
    for (const escape of m.escapes) failures.push(`${at}: outside its panel: ${escape}`);
    for (const row of m.heads) {
      const offsets = row
        .slice(row.indexOf(':') + 1)
        .split('/')
        .map(Number);
      if (Math.max(...offsets) - Math.min(...offsets) > 0.5) {
        failures.push(`${at}: head rows at different heights, ${row}`);
      }
    }
    for (const gap of m.gaps) {
      if (Number(gap.slice(gap.lastIndexOf(' ') + 1)) < 4) {
        failures.push(`${at}: value and caption touch, ${gap}`);
      }
    }
  }

  // An opened panel's evidence: its heading sits above its body, never on one line with it.
  await window.locator('.cdt-fr-panel').first().click();
  const evidence = await window.evaluate(() => {
    const label = document.querySelector('.cdt-fr-evidence-label')?.getBoundingClientRect();
    const body = document.querySelector('.cdt-fr-evidence-body')?.getBoundingClientRect();
    return label === undefined || body === undefined ? null : body.top - label.bottom;
  });
  // eslint-disable-next-line no-console -- the measurement is the deliverable
  console.log(`reveal evidence: body below its heading by ${String(evidence)}`);
  if (evidence === null || evidence < 2) {
    failures.push(`reveal: the evidence body is not below its heading (${String(evidence)})`);
  }
  await window.locator('.cdt-fr-panel').first().click();
  return failures;
}

interface TermsMeasure {
  readonly rows: number;
  readonly tableWidth: number;
  readonly labelWidest: number;
  readonly valueWidest: number;
  readonly valueRights: number[];
  readonly nearest: number;
}

/** Profile's terms table: each label's text, each figure's text, and the table that holds them. */
function measureTerms(window: Page): Promise<TermsMeasure | null> {
  return window.evaluate(() => {
    const table = document.querySelector<HTMLTableElement>('.cdt-profile-terms');
    if (table === null) return null;
    const text = (el: Element | null): DOMRect | null => {
      if (el === null) return null;
      const range = document.createRange();
      range.selectNodeContents(el);
      return range.getBoundingClientRect();
    };
    const pairs = [...table.rows].map((tr) => ({
      label: text(tr.querySelector('th')),
      value: text(tr.querySelector('td')),
    }));
    const labels = pairs.flatMap((p) => (p.label === null ? [] : [p.label]));
    const values = pairs.flatMap((p) => (p.value === null ? [] : [p.value]));
    return {
      rows: pairs.length,
      tableWidth: table.getBoundingClientRect().width,
      labelWidest: Math.max(...labels.map((r) => r.width)),
      valueWidest: Math.max(...values.map((r) => r.width)),
      valueRights: values.map((r) => r.right),
      nearest: Math.min(
        ...pairs.map((p) =>
          p.label === null || p.value === null ? Infinity : p.value.left - p.label.right,
        ),
      ),
    };
  });
}

test.describe.serial('the band, Profile and the reveal, laid out', () => {
  test.skip(!existsSync(coreBinary), NO_CORE);

  let home = '';
  let userData = '';
  let app: ElectronApplication | null = null;
  let window: Page;

  test.afterAll(async () => {
    await app?.close();
  });

  test('the reveal keeps every panel inside its border, its heads level and its value off its caption', async () => {
    test.setTimeout(300_000);
    home = seedHome(REPOS);
    // History behind each repository, so the reveal has a span and a level to show.
    REPOS.forEach((repo, index) => {
      commitBackdated(home, repo, 40 + index * 70);
    });
    userData = mkdtempSync(path.join(tmpdir(), 'codotheca-layout-'));
    app = await launch(home, userData);
    window = await openWindow(app);
    await window.waitForSelector('.cdt-fr-view--roots', { timeout: 60_000 });
    await window.getByRole('button', { name: 'DIG', exact: true }).click();
    const goOn = window.getByRole('button', { name: 'GO ON', exact: true });
    await goOn.waitFor({ state: 'visible', timeout: 150_000 });

    const failures = await revealFailures(app, window);

    await setContentWidth(app, window, 1440);
    await goOn.click();
    const notNow = window.getByRole('button', { name: 'NOT NOW', exact: true });
    await notNow.waitFor({ state: 'visible', timeout: 60_000 });
    await notNow.click();
    await expect(window.locator('.cdt-card')).toHaveCount(REPOS.length, { timeout: 60_000 });
    expect(failures).toEqual([]);
  });

  test('the day-one band stays inside itself at every width', async () => {
    test.setTimeout(120_000);
    if (app === null) throw new Error('no app: the reveal case did not launch one');
    await expect(window.locator('.cdt-band')).toBeVisible({ timeout: 60_000 });
    expect(await bandAtEveryWidth(app, window, 'day-one', false)).toEqual([]);
  });

  test('the band with drawn rings stays inside itself at every width', async () => {
    test.setTimeout(180_000);
    await app?.close();
    app = null;
    backdateDayOne(userData);
    app = await launch(home, userData);
    window = await openWindow(app);
    await expect(window.locator('.cdt-band')).toBeVisible({ timeout: 60_000 });
    await expect(window.locator('.cdt-band-ring-glyph')).toHaveCount(3, { timeout: 60_000 });
    expect(await bandAtEveryWidth(app, window, 'rings', true)).toEqual([]);
  });

  test('Profile sets each term figure right-aligned beside its label', async () => {
    test.setTimeout(120_000);
    if (app === null) throw new Error('no app: the band case did not launch one');
    await setContentWidth(app, window, 1440);
    await window.locator('.cdt-band-level-number').click();
    await expect(window.locator('.cdt-profile-terms')).toBeVisible({ timeout: 30_000 });
    const m = await measureTerms(window);
    // eslint-disable-next-line no-console -- the measurement is the deliverable
    console.log(`profile 1440: ${JSON.stringify(m)}`);
    if (SHOTS !== undefined)
      await window.screenshot({ path: path.join(SHOTS, 'profile-1440.png') });
    expect(m, 'no terms table').not.toBeNull();
    if (m === null) return;
    expect(m.rows, 'a table of fewer than two rows proves nothing').toBeGreaterThan(1);
    // Right-aligned: every figure ends on one edge.
    expect(Math.max(...m.valueRights) - Math.min(...m.valueRights)).toBeLessThanOrEqual(0.5);
    // Content-width: the table is its widest label and widest figure, and a gutter between.
    expect(m.tableWidth).toBeLessThanOrEqual(m.labelWidest + m.valueWidest + 64);
    // No figure touches its label.
    expect(m.nearest).toBeGreaterThanOrEqual(8);
  });
});

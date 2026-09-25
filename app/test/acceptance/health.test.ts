/**
 * [p3] Acceptance: §30 — the two criteria whose surfaces are the app's.
 *
 * `AC-P3-30-1`'s second half: **with every check off, no surface renders a health number for any
 * project.** The core half — that a reading with nothing eligible is `absent`, with no checks, no
 * basis and no `scoredOpen` — is `core/tests/acceptance_p3_health.rs`.
 *
 * `AC-P3-30-3`: the four outcomes in four distinct rendered forms, none of the three withheld
 * ones taking the passing one's shape.
 *
 * **Asserted against the producer, and tied to the page by a source read.** This file is in the
 * `node` vitest project, whose tsconfig sets no `--jsx`, so it cannot import the `.tsx`
 * component — and moving it to the `dom` project would move it out of `test/acceptance/`. The
 * strings the tab renders are `formFor`'s and `basisLine`'s verbatim, and the last assertion in
 * each test reads `HealthTab.tsx` to prove that is still true: a tab that started composing its
 * own words would fail here rather than drift.
 *
 * `HealthTab.test.tsx` asserts the same four forms **through a real render**, in the `dom`
 * project. Both reach `npm run acceptance`'s vitest writer, which reads the whole workspace run.
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

import type {
  DebtItem,
  HealthCheck,
  HealthReading,
  HealthSummary,
} from '../../src/generated/protocol';
import { litCounts } from '../../src/renderer/decay/lit';
import { basisLine, formFor, openLine } from '../../src/renderer/project/health/checkForms';
import { checkLabel } from '../../src/renderer/project/health/labels';
import { tabsFor } from '../../src/renderer/project/tabs';
import { detailFixture, rowFixture } from '../../src/renderer/project/testFixtures';
import { rankOf, toShelfRow } from '../../src/renderer/shelf/row';

const NOW = 1_700_000_000;

const TAB_SOURCE = readFileSync(
  fileURLToPath(new URL('../../src/renderer/project/health/HealthTab.tsx', import.meta.url)),
  'utf8',
);

/** The core's own all-checks-off output, which the core's half of `AC-P3-30-1` pins. */
const ALL_CHECKS_OFF = JSON.parse(
  readFileSync(
    fileURLToPath(new URL('../../../protocol/health/all-checks-off.json', import.meta.url)),
    'utf8',
  ),
) as { reading: HealthReading; summary: HealthSummary; items: DebtItem[] };

/** Every string the tab would draw for this reading, in the order it draws them. */
function surfaceOf(reading: HealthReading): string[] {
  const out: string[] = [];
  const open = openLine(reading.scoredOpen, reading.basis);
  if (open !== null) out.push(open);
  const coverage = basisLine(reading.basis, reading.state);
  if (coverage !== null) out.push(coverage);
  for (const check of reading.checks) {
    const form = formFor(check);
    out.push(checkLabel(check.id), form.word);
    if (form.detail !== '') out.push(form.detail);
  }
  return out;
}

describe('health', () => {
  it('AC-P3-30-1 with every check off no surface renders a health number', () => {
    // What the CORE hands down with every check off — the page's reading, the shelf's summary and
    // the item list — read from the fixture `core/tests/acceptance_p3_health.rs` holds the real
    // producer's output equal to. A reading built here instead would test the renderer against
    // this file's own idea of the core.
    const { reading, summary, items } = ALL_CHECKS_OFF;
    const projects = [1, 2, 3];
    let inspected = 0;
    for (const project of projects) {
      // The page: §30.7 mounts no `HEALTH` tab on an `absent` reading.
      const tabs = tabsFor(detailFixture({ health: reading })).map((tab) => tab.id);
      expect(tabs).not.toContain('health');
      inspected += 1;
      // The tab's own strings, were it drawn: none at all.
      const surface = surfaceOf(reading);
      expect(surface).toEqual([]);
      inspected += 1;
      // The shelf: §35's rank reads no number off the summary, so the row sorts to the tail.
      expect(rankOf(toShelfRow(rowFixture({ healthSummary: summary })))).toBeNull();
      inspected += 1;
      // The hero: §33 lights a layer from an open item and is handed none.
      expect(litCounts(items).size).toBe(0);
      inspected += 1;
      // eslint-disable-next-line no-console
      console.log(
        `AC-P3-30-1 project ${String(project)}: tabs ${JSON.stringify(tabs)}, tab strings []`,
      );
    }
    // eslint-disable-next-line no-console
    console.log(`AC-P3-30-1 surfaces inspected: ${String(inspected)}`);
    expect(inspected).toBe(projects.length * 4);
    expect(inspected).toBeGreaterThan(0);

    // And the tab draws exactly those strings, so the emptiness above is the page's emptiness.
    expect(TAB_SOURCE).toContain('{open}');
    expect(TAB_SOURCE).toContain('{coverage}');
  });

  it('AC-P3-30-3 the four outcomes reach the page in four distinct rendered forms', () => {
    const checks: HealthCheck[] = [
      { id: 'missing_readme', outcome: 'ok', unknownReason: null },
      { id: 'missing_license', outcome: 'unknown', unknownReason: 'notRead' },
      { id: 'missing_tests', outcome: 'off', unknownReason: null },
      { id: 'ci_red', outcome: 'notApplicable', unknownReason: null },
    ];
    const rendered = checks.map((check) => {
      const form = formFor(check);
      return `${form.word} ${form.detail}`.trim();
    });
    // eslint-disable-next-line no-console
    console.log(`AC-P3-30-3 rendered forms: ${JSON.stringify(rendered)}`);
    expect(rendered).toHaveLength(4);
    expect(new Set(rendered).size).toBe(4);

    const passing = rendered[0];
    expect(passing).not.toBe('');
    for (const other of rendered.slice(1)) {
      expect(other).not.toBe(passing);
    }

    // The tab draws `form.word` and `form.detail` verbatim, and hangs a per-row attribute off the
    // outcome so §33 cannot collapse two of them into one appearance without moving this first.
    expect(TAB_SOURCE).toContain('{form.word}');
    expect(TAB_SOURCE).toContain('{form.detail}');
    expect(TAB_SOURCE).toContain('data-outcome={check.outcome}');
    // …and names each check from the one label table, as `surfaceOf` above does.
    expect(TAB_SOURCE).toContain('{checkLabel(check.id)}');

    // The basis renders beside them, dated from its one owner.
    const basis = { ran: 1, eligible: 2, unknown: 1, off: 1, notApplicable: 1, observedAt: NOW };
    expect(basisLine(basis, 'live')).not.toBeNull();
  });
});

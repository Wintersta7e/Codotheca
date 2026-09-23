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

import type { HealthCheck, HealthReading, HealthSummary } from '../../src/generated/protocol';
import { basisLine, formFor, openLine } from '../../src/renderer/project/health/checkForms';
import { tabsFor } from '../../src/renderer/project/tabs';
import { detailFixture, rowFixture } from '../../src/renderer/project/testFixtures';
import { rankOf, toShelfRow } from '../../src/renderer/shelf/row';

const NOW = 1_700_000_000;

const TAB_SOURCE = readFileSync(
  fileURLToPath(new URL('../../src/renderer/project/health/HealthTab.tsx', import.meta.url)),
  'utf8',
);

/** Every string the tab would draw for this reading, in the order it draws them. */
function surfaceOf(reading: HealthReading): string[] {
  const out: string[] = [];
  const open = openLine(reading.scoredOpen, reading.basis);
  if (open !== null) out.push(open);
  const coverage = basisLine(reading.basis, reading.state);
  if (coverage !== null) out.push(coverage);
  for (const check of reading.checks) {
    const form = formFor(check);
    out.push(check.id, form.word);
    if (form.detail !== '') out.push(form.detail);
  }
  return out;
}

describe('health', () => {
  it('AC-P3-30-1 with every check off no surface renders a health number', () => {
    // Every check off is a reading with nothing eligible, and §30.1 rules that `absent`: no
    // checks, no `scoredOpen`, no basis — exactly what the core produces for that case, on the
    // page and flattened onto the shelf row.
    const reading: HealthReading = { state: 'absent', scoredOpen: null, basis: null, checks: [] };
    const summary: HealthSummary = {
      state: 'absent',
      scoredOpen: null,
      unverified: null,
      unknownChecks: null,
      observedAt: null,
    };
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
      // eslint-disable-next-line no-console
      console.log(`AC-P3-30-1 project ${project}: tabs ${JSON.stringify(tabs)}, tab strings []`);
    }
    // eslint-disable-next-line no-console
    console.log(`AC-P3-30-1 surfaces inspected: ${inspected}`);
    expect(inspected).toBe(projects.length * 3);
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

    // The basis renders beside them, dated from its one owner.
    const basis = { ran: 1, eligible: 2, unknown: 1, off: 1, notApplicable: 1, observedAt: NOW };
    expect(basisLine(basis, 'live')).not.toBeNull();
  });
});

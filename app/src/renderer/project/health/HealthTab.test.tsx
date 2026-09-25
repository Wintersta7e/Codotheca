/**
 * [p3] §30.4 and §30.7 — what the tab may never claim, and the two causes of `off` rendered as
 * two sentences.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

afterEach(cleanup);

import type { DebtSource, HealthReading, Settings } from '../../../generated/protocol';
import { GRANT_ASK_ACTION } from './GrantAsk';
import { HealthTab } from './HealthTab';
import { SOURCE_LABELS } from './labels';

const NOW = 1_700_000_000;

function settings(over: Partial<Settings> = {}): Settings {
  return {
    effectsTier: 'auto',
    reducedMotionOverride: false,
    autostart: false,
    residentShortcut: null,
    roastEnabled: true,
    logLevel: 'info',
    installRootId: null,
    contentScanEnabled: true,
    healthChecks: [
      { check: 'todo_marker', enabled: true },
      { check: 'missing_readme', enabled: true },
      { check: 'missing_license', enabled: true },
    ],
    ...over,
  };
}

function reading(over: Partial<HealthReading> = {}): HealthReading {
  return {
    state: 'live',
    scoredOpen: 1,
    basis: {
      ran: 2,
      eligible: 3,
      unknown: 1,
      off: 0,
      notApplicable: 0,
      observedAt: NOW - 30,
    },
    checks: [
      { id: 'missing_readme', outcome: 'ok', unknownReason: null },
      { id: 'missing_license', outcome: 'failed', unknownReason: null },
      { id: 'missing_tests', outcome: 'unknown', unknownReason: 'notRunYet' },
    ],
    ...over,
  };
}

function draw(r: HealthReading, s: Settings = settings(), onGrant = vi.fn()): HTMLElement {
  // Each draw replaces the last: two mounted copies would make every `getByTestId` ambiguous,
  // and a query that resolved the wrong one would assert about a tab the test is not describing.
  cleanup();
  const { container } = render(
    <HealthTab reading={r} settings={s} now={NOW} onGrantSourceReading={onGrant} />,
  );
  const root = container.querySelector('[data-testid="cp-health"]');
  if (root === null) throw new Error('the tab rendered no root');
  return root as HTMLElement;
}

describe('§30.4 what the tab may never claim', () => {
  it('AC-P3-30-17 a live reading with ran zero renders no count', () => {
    const root = draw(
      reading({
        scoredOpen: null,
        basis: { ran: 0, eligible: 3, unknown: 3, off: 0, notApplicable: 0, observedAt: NOW - 10 },
        checks: [
          { id: 'missing_readme', outcome: 'unknown', unknownReason: 'notRunYet' },
          { id: 'missing_license', outcome: 'unknown', unknownReason: 'notRunYet' },
          { id: 'missing_tests', outcome: 'unknown', unknownReason: 'notRunYet' },
        ],
      }),
    );
    const surfaces = [
      root.querySelector('[data-testid="cp-health-open"]'),
      root.querySelector('[data-testid="cp-health-basis"]'),
      root.querySelector('[data-testid="cp-health-age"]'),
    ];
    // eslint-disable-next-line no-console
    console.log(
      `AC-P3-30-17 surfaces inspected: ${String(surfaces.length)}; texts: ${JSON.stringify(
        surfaces.map((n) => n?.textContent ?? null),
      )}`,
    );
    expect(surfaces.length).toBeGreaterThan(0);
    expect(root.querySelector('[data-testid="cp-health-open"]')).toBeNull();
    expect(root.textContent ?? '').not.toMatch(/\b0 of 0\b/u);
    // The basis alone still renders, so the tab is not silent about coverage.
    expect(root.querySelector('[data-testid="cp-health-basis"]')?.textContent ?? '').toMatch(
      /no check has run yet/u,
    );
  });

  it('every unknown check renders its reason', () => {
    const root = draw(reading());
    const unknowns = root.querySelectorAll('[data-outcome="unknown"]');
    expect(unknowns.length).toBeGreaterThan(0);
    for (const node of unknowns) {
      expect(node.querySelector('.cp-health-check-detail')?.textContent ?? '').not.toBe('');
    }
  });

  it('says nothing open only when unknown is zero', () => {
    const withUnknown = draw(reading({ scoredOpen: 0 }));
    expect(withUnknown.textContent ?? '').toMatch(/nothing open in the checks that ran/u);

    const clean = draw(
      reading({
        scoredOpen: 0,
        basis: { ran: 3, eligible: 3, unknown: 0, off: 0, notApplicable: 0, observedAt: NOW - 5 },
      }),
    );
    expect(clean.querySelector('[data-testid="cp-health-open"]')?.textContent).toBe('nothing open');
  });

  it('the words clean healthy none and all appear in no rendered string or accessible name', () => {
    const root = draw(
      reading({
        checks: [
          { id: 'missing_readme', outcome: 'ok', unknownReason: null },
          { id: 'dependency_advisory', outcome: 'ok', unknownReason: null },
          { id: 'missing_tests', outcome: 'off', unknownReason: null },
          { id: 'ci_red', outcome: 'notApplicable', unknownReason: null },
          { id: 'no_release', outcome: 'unknown', unknownReason: 'needsAccount' },
        ],
      }),
    );
    const text = root.textContent ?? '';
    const names = [...root.querySelectorAll('*')]
      .map((n) => n.getAttribute('aria-label') ?? '')
      .join(' ');
    const classes = [...root.querySelectorAll('*')].map((n) => n.className).join(' ');
    for (const banned of ['clean', 'healthy', 'none', 'all']) {
      const pattern = new RegExp(`\\b${banned}\\b`, 'iu');
      expect(text).not.toMatch(pattern);
      expect(names).not.toMatch(pattern);
      expect(classes).not.toMatch(pattern);
    }
  });

  it('a frozen reading always renders its age and says the store is away', () => {
    const root = draw(
      reading({ state: 'frozen', basis: { ...reading().basis!, observedAt: NOW - 5 } }),
    );
    expect(root.querySelector('[data-testid="cp-health-age"]')?.textContent ?? '').toMatch(
      /under glass/u,
    );
  });
});

describe('each check row reads as text', () => {
  it('separates the check, its word and its detail in the text a reader gets', () => {
    // The tab carries no stylesheet for these spans, so the text is all a reader has to go on.
    const root = draw(
      reading({
        checks: [
          { id: 'missing_license', outcome: 'failed', unknownReason: null },
          { id: 'missing_tests', outcome: 'unknown', unknownReason: 'notRunYet' },
          { id: 'todo_marker', outcome: 'off', unknownReason: null },
        ],
      }),
      settings({ healthChecks: [{ check: 'todo_marker', enabled: false }] }),
    );
    const texts = [...root.querySelectorAll('.cp-health-check')].map((li) => li.textContent);
    expect(texts).toEqual([
      `${SOURCE_LABELS.missing_license.check} · OPEN`,
      `${SOURCE_LABELS.missing_tests.check} · UNKNOWN · scheduled, and has not run yet`,
      `${SOURCE_LABELS.todo_marker.check} · SWITCHED OFF · you switched this check off`,
    ]);
  });

  it('names each check in words and keeps the raw id out of sight', () => {
    const root = draw(reading());
    const rows = [...root.querySelectorAll('.cp-health-check')];
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      const id = row.getAttribute('data-check') ?? '';
      expect(id, 'the row carries its id for code to find').not.toBe('');
      expect(row.textContent ?? '', id).not.toContain(id);
      expect(row.querySelector('.cp-health-check-name')?.textContent).toBe(
        SOURCE_LABELS[id as keyof typeof SOURCE_LABELS].check,
      );
    }
  });
});

describe('a check is named for what it looks at, never for what it finds', () => {
  it('a passing check does not state the problem it looks for', () => {
    const root = draw(
      reading({
        checks: [
          { id: 'missing_readme', outcome: 'ok', unknownReason: null },
          { id: 'ci_red', outcome: 'ok', unknownReason: null },
        ],
      }),
    );
    const texts = [...root.querySelectorAll('.cp-health-check')].map((li) => li.textContent);
    // `No README · PASSED` tells a project that has a README that it has none.
    expect(texts).toEqual(['README · PASSED', 'CI green · PASSED']);
  });

  it('names a check this build does not know by its id rather than leaving it blank', () => {
    // A newer core's tenth source: the table has no words for it, and a blank name is worse
    // than the id.
    const tenth = 'tenth_source' as unknown as DebtSource;
    const root = draw(reading({ checks: [{ id: tenth, outcome: 'failed', unknownReason: null }] }));
    expect(root.querySelector('.cp-health-check-name')?.textContent).toBe('tenth_source');
  });
});

describe('R142 the in-context grant ask', () => {
  it('the word and the basis line follow the cause of off, and the switch wins', () => {
    const offMarker = reading({
      scoredOpen: 0,
      basis: { ran: 1, eligible: 1, unknown: 0, off: 1, notApplicable: 0, observedAt: NOW - 5 },
      checks: [
        { id: 'missing_readme', outcome: 'ok', unknownReason: null },
        { id: 'todo_marker', outcome: 'off', unknownReason: null },
      ],
    });
    const drawn = (enabled: boolean, contentScanEnabled: boolean): [string, string] => {
      const root = draw(
        offMarker,
        settings({ contentScanEnabled, healthChecks: [{ check: 'todo_marker', enabled }] }),
      );
      const row = root.querySelector('.cp-health-check[data-check="todo_marker"]');
      return [
        row?.querySelector('.cp-health-check-word')?.textContent ?? '',
        root.querySelector('[data-testid="cp-health-basis"]')?.textContent ?? '',
      ];
    };

    // The user's act: the switch is off.
    const [switchedWord, switchedBasis] = drawn(false, true);
    expect(switchedWord).toBe('SWITCHED OFF');
    expect(switchedBasis).toContain('1 switched off');
    expect(switchedBasis).not.toMatch(/grant/u);

    // Not the user's act: the switch is on and the grant was never given.
    const [grantWord, grantBasis] = drawn(true, false);
    expect(grantWord).toBe('NEEDS A GRANT');
    expect(grantBasis).toContain('1 needs a grant');
    expect(grantBasis).not.toMatch(/switched off/u);

    // Both apply: the switch wins.
    const [bothWord, bothBasis] = drawn(false, false);
    expect(bothWord).toBe('SWITCHED OFF');
    expect(bothBasis).toContain('1 switched off');
    expect(bothBasis).not.toMatch(/grant/u);
  });

  it('the two causes of off render two different sentences', () => {
    const switched = draw(
      reading({
        checks: [{ id: 'todo_marker', outcome: 'off', unknownReason: null }],
      }),
      settings({ healthChecks: [{ check: 'todo_marker', enabled: false }] }),
    );
    const switchedText = switched.textContent ?? '';

    const ungranted = draw(
      reading({
        checks: [{ id: 'todo_marker', outcome: 'off', unknownReason: null }],
      }),
      settings({
        contentScanEnabled: false,
        healthChecks: [{ check: 'todo_marker', enabled: true }],
      }),
    );
    const ungrantedText = ungranted.textContent ?? '';
    // eslint-disable-next-line no-console
    console.log(`switchedOff: ${switchedText}\ngrantMissing: ${ungrantedText}`);
    expect(switchedText).not.toBe(ungrantedText);
  });

  it('a check switched off offers the switch and never the grant ask', () => {
    const root = draw(
      reading({ checks: [{ id: 'todo_marker', outcome: 'off', unknownReason: null }] }),
      settings({
        contentScanEnabled: false,
        healthChecks: [{ check: 'todo_marker', enabled: false }],
      }),
    );
    expect(root.querySelector('[data-testid="cp-health-ask"]')).toBeNull();
    expect(root.textContent ?? '').toMatch(/switched this check off/u);
  });

  it('an ungranted content scan offers the ask and granting it calls settings.set once', () => {
    const onGrant = vi.fn();
    const root = draw(
      reading({ checks: [{ id: 'todo_marker', outcome: 'off', unknownReason: null }] }),
      settings({
        contentScanEnabled: false,
        healthChecks: [{ check: 'todo_marker', enabled: true }],
      }),
      onGrant,
    );
    const ask = root.querySelector('[data-testid="cp-health-ask"]');
    expect(ask).not.toBeNull();
    fireEvent.click(screen.getByText(GRANT_ASK_ACTION));
    expect(onGrant).toHaveBeenCalledTimes(1);
  });
});

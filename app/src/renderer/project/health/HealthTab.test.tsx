/**
 * [p3] §30.4 and §30.7 — what the tab may never claim, and the two causes of `off` rendered as
 * two sentences.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

afterEach(cleanup);

import type { HealthReading, Settings } from '../../../generated/protocol';
import { GRANT_ASK_ACTION } from './GrantAsk';
import { HealthTab } from './HealthTab';

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
      `AC-P3-30-17 surfaces inspected: ${surfaces.length}; texts: ${JSON.stringify(
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

describe('R142 the in-context grant ask', () => {
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

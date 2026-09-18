/**
 * [p3] §30.4 — **four distinct forms, and none of the latter three is the passing one.**
 */
import { describe, expect, it } from 'vitest';

import type { HealthBasis, HealthCheck, Settings } from '../../../generated/protocol';
import { basisLine, formFor, offCause, openLine } from './checkForms';

function check(over: Partial<HealthCheck> = {}): HealthCheck {
  return { id: 'missing_readme', outcome: 'ok', unknownReason: null, ...over };
}

function basis(over: Partial<HealthBasis> = {}): HealthBasis {
  return {
    ran: 2,
    eligible: 3,
    unknown: 1,
    off: 0,
    notApplicable: 0,
    observedAt: 1_700_000_000,
    ...over,
  };
}

const SETTINGS: Settings = {
  effectsTier: 'auto',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
  installRootId: null,
  contentScanEnabled: false,
  healthChecks: [
    { check: 'todo_marker', enabled: true },
    { check: 'missing_readme', enabled: true },
  ],
};

describe('§30.4 the four outcome forms', () => {
  it('AC-P3-30-3 off, not applicable, unknown and ok render in four distinct forms', () => {
    const forms = (['ok', 'off', 'notApplicable', 'unknown'] as const).map((outcome) =>
      formFor(check({ outcome, unknownReason: outcome === 'unknown' ? 'notRunYet' : null })),
    );
    const strings = forms.map((f) => `${f.word} ${f.detail}`.trim());
    // Printed, because the claim is about the strings a user reads and not about the enum.
    // eslint-disable-next-line no-console
    console.log(`AC-P3-30-3 rendered forms: ${JSON.stringify(strings)}`);
    expect(strings).toHaveLength(4);
    expect(new Set(strings).size).toBe(4);

    const passing = strings[0];
    for (const other of strings.slice(1)) {
      expect(other).not.toBe(passing);
    }
    // And `failed` is a fifth word, distinct from all four: a check with an open item may not
    // read as any of the three withheld forms either.
    expect(strings).not.toContain(formFor(check({ outcome: 'failed' })).word);
  });

  it('names every unknown with §30.3s vocabulary and never collapses them to one string', () => {
    const reasons = [
      'needsAccount',
      'notSynced',
      'notRead',
      'notObserved',
      'notRunYet',
      'unreachable',
    ] as const;
    const details = reasons.map(
      (unknownReason) => formFor(check({ outcome: 'unknown', unknownReason })).detail,
    );
    expect(new Set(details).size).toBe(reasons.length);
    for (const detail of details) expect(detail).not.toBe('');
  });

  it('renders no count when ran is zero, and the basis alone', () => {
    expect(openLine(0, basis({ ran: 0, eligible: 3, unknown: 3 }))).toBeNull();
    expect(basisLine(basis({ ran: 0, eligible: 3, unknown: 3 }), 'live')).toMatch(
      /no check has run yet/u,
    );
    // The bare zero this rule exists to prevent must appear nowhere in that line.
    expect(basisLine(basis({ ran: 0, eligible: 3, unknown: 3 }), 'live')).not.toMatch(
      /\b0 of 0\b/u,
    );
  });

  it('says nothing outstanding only when unknown is zero', () => {
    expect(openLine(0, basis({ unknown: 0, eligible: 2 }))).toBe('nothing open');
    expect(openLine(0, basis({ unknown: 1 }))).toBe('nothing open in the checks that ran');
    expect(openLine(3, basis())).toBe('3 open');
    // A reading with no basis carries no count at all.
    expect(openLine(null, null)).toBeNull();
  });
});

describe('R142 the two causes of off', () => {
  it('reads a switched-off check as the switch, whatever the grant says', () => {
    const settings: Settings = {
      ...SETTINGS,
      contentScanEnabled: false,
      healthChecks: [{ check: 'todo_marker', enabled: false }],
    };
    expect(offCause(check({ id: 'todo_marker', outcome: 'off' }), settings)).toBe('switchedOff');
  });

  it('reads an ungranted content scan as the grant when the switch is on', () => {
    expect(offCause(check({ id: 'todo_marker', outcome: 'off' }), SETTINGS)).toBe('grantMissing');
  });

  it('offers the grant to no other source', () => {
    expect(offCause(check({ id: 'missing_readme', outcome: 'off' }), SETTINGS)).toBe('switchedOff');
  });
});

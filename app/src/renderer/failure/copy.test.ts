import { describe, expect, it } from 'vitest';
import type { LedgerCounts, StartupFailure } from '../../shared/startupFailure';
import {
  corruptLedger,
  failureCopy,
  forceIsOffered,
  FORCE_AVAILABLE_AFTER_MS,
  logPathNote,
  shuttingDownNote,
  type FailureFact,
} from './copy';

const COUNTS: LedgerCounts = {
  projects: 212,
  notes: 14,
  sessions: 96,
  collections: 3,
  roots: 2,
  xpEvents: 410,
  launchTargets: 6,
};

const corrupt: Extract<StartupFailure, { kind: 'corrupt_index' }> = {
  kind: 'corrupt_index',
  quarantinedAt: 1_700_000_000,
  gapStartedAt: 1_699_996_400,
  gapCountsRecoverable: true,
  reDerivable: COUNTS,
  restorable: COUNTS,
};

describe('the schema-from-the-future window', () => {
  it('names both numbers, because "please update" without them is unactionable', () => {
    const copy = failureCopy({ kind: 'schema_from_future', onDisk: 9, supported: 5 });
    const body = copy.body.join(' ');
    expect(body).toContain('9');
    expect(body).toContain('5');
    expect(copy.primary).toBe('QUIT');
    expect(copy.secondary).toBeNull();
  });
});

describe('the failed-migration window', () => {
  it('names the schema it was restored to and the restore time, never "an error occurred"', () => {
    const copy = failureCopy({
      kind: 'migration_failed',
      version: 4,
      name: 'sessions',
      restoredTo: 3,
      restoredAt: 1_700_000_000,
    });
    const body = copy.body.join(' ');
    expect(body).toContain('3');
    expect(body).not.toMatch(/an error occurred/i);
    expect(body).toMatch(/\d{4}/); // the restore time carries its date
    expect(copy.primary).toBe('QUIT');
  });
});

describe('the corrupt-index window', () => {
  it('offers REBUILD over QUIT', () => {
    const copy = failureCopy(corrupt);
    expect(copy.primary).toBe('REBUILD');
    expect(copy.secondary).toBe('QUIT');
  });

  it('draws exactly three blocks, the third being the one §1.12 never states', () => {
    const blocks = corruptLedger(corrupt);
    expect(blocks.map((b) => b.label)).toEqual([
      'RE-DERIVED FROM DISK',
      'RESTORED FROM THE SIDECAR',
      'LOST IN THE GAP',
    ]);
  });

  it('counts every ledger §11.2a enumerates, in both recoverable blocks', () => {
    const [reDerived, restored] = corruptLedger(corrupt);
    expect(reDerived?.lines.join(' ')).toContain('212 projects');
    expect(restored?.lines.join(' ')).toContain('410 XP events');
    expect(restored?.lines.join(' ')).toContain('14 notes');
  });

  it('gives the gap its start time', () => {
    const gap = corruptLedger(corrupt)[2];
    expect(gap?.lines.join(' ')).toMatch(/\d{4}/);
  });

  it('says the gap start is unknown rather than printing a zero or a date', () => {
    const gap = corruptLedger({ ...corrupt, gapStartedAt: null })[2];
    const text = gap?.lines.join(' ') ?? '';
    expect(text).toContain('not known');
    expect(text).not.toMatch(/1970|\b0\b/);
  });

  it('prints no figure at all when the gap cannot be counted', () => {
    const gap = corruptLedger({ ...corrupt, gapCountsRecoverable: false })[2];
    const text = gap?.lines.join(' ') ?? '';
    expect(text).toContain('cannot be counted');
    expect(text).not.toMatch(/\b\d+\s+(notes|sessions|projects)\b/);
  });

  // Plan 17 deviation D4: `from_index_error` genuinely cannot know either ledger, so both
  // blocks are nullable on the wire. `0 projects` there is a claim about what was lost, made
  // on the one screen where unknown-as-zero costs the most.
  it('prints no figure for a block the report could not count, and says so', () => {
    const blocks = corruptLedger({ ...corrupt, reDerivable: null, restorable: null });
    for (const block of [blocks[0], blocks[1]]) {
      const text = block?.lines.join(' ') ?? '';
      expect(text).toContain('not known');
      expect(text).not.toMatch(/\d/);
    }
  });

  it('states a measured empty ledger as nothing, and still prints no zeroed nouns', () => {
    const empty: LedgerCounts = {
      projects: 0,
      notes: 0,
      sessions: 0,
      collections: 0,
      roots: 0,
      xpEvents: 0,
      launchTargets: 0,
    };
    const text = corruptLedger({ ...corrupt, restorable: empty })[1]?.lines.join(' ') ?? '';
    expect(text).not.toMatch(/\b0\s/);
    expect(text.length).toBeGreaterThan(0);
  });
});

describe('the still-shutting-down window', () => {
  it('counts real elapsed seconds, never a spinner', () => {
    expect(shuttingDownNote(1_000, 8_000)).toBe('still shutting down · 7s');
    expect(shuttingDownNote(1_000, 1_000)).toBe('still shutting down · 0s');
    expect(shuttingDownNote(9_000, 1_000)).toBe('still shutting down · 0s');
  });

  it('keeps WAIT as the filled primary and FORCE as the secondary', () => {
    const copy = failureCopy({ kind: 'still_shutting_down', startedAtMs: 0 });
    expect(copy.primary).toBe('WAIT');
    expect(copy.secondary).toBe('FORCE');
  });

  it('offers FORCE only after ten seconds of actual waiting', () => {
    expect(FORCE_AVAILABLE_AFTER_MS).toBe(10_000);
    expect(forceIsOffered(0, 9_999)).toBe(false);
    expect(forceIsOffered(0, 10_000)).toBe(true);
  });
});

describe('every window', () => {
  const facts: readonly FailureFact[] = [
    { kind: 'schema_from_future', onDisk: 9, supported: 5 },
    { kind: 'migration_failed', version: 4, name: 'sessions', restoredTo: 3, restoredAt: 1 },
    corrupt,
    { kind: 'still_shutting_down', startedAtMs: 0 },
  ];

  it('has an eyebrow, a headline, a body and a primary', () => {
    expect(facts).toHaveLength(4);
    for (const fact of facts) {
      const copy = failureCopy(fact);
      expect(copy.eyebrow.length).toBeGreaterThan(0);
      expect(copy.headline.length).toBeGreaterThan(0);
      expect(copy.body.length).toBeGreaterThan(0);
      expect(copy.primary.length).toBeGreaterThan(0);
    }
  });

  it('never names a destructive operation phase 1 does not have', () => {
    for (const fact of facts) {
      const copy = failureCopy(fact);
      const all = [copy.eyebrow, copy.headline, copy.primary, copy.secondary ?? '', ...copy.body];
      for (const s of all) expect(s).not.toMatch(/forget|uninstall|push|checkout/i);
    }
  });

  it('shows the log path as a path and not as a sentence', () => {
    expect(logPathNote('/data/codotheca.log')).toBe('LOG · /data/codotheca.log');
  });
});

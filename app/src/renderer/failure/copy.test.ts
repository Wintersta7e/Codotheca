import { describe, expect, it } from 'vitest';
import type { SidecarReport, StartupFailure } from '../../shared/startupFailure';
import {
  corruptLedger,
  failureCopy,
  forceIsOffered,
  FORCE_AVAILABLE_AFTER_MS,
  logPathNote,
  offersRebuild,
  shuttingDownNote,
  SIDECAR_COUNT_NOUNS,
  type FailureFact,
} from './copy';

// The sidecar keys its counts as the core does: one figure per key, each distinct, so a line
// can only have come from its own key. `1` is one of them, for the singular.
const COUNTS: Readonly<Record<string, number>> = Object.fromEntries(
  Object.keys(SIDECAR_COUNT_NOUNS).map((key, index) => [key, index === 0 ? 1 : 100 + index]),
);

type CorruptFact = Extract<StartupFailure, { kind: 'corrupt_index' }>;

const corrupt: CorruptFact = {
  kind: 'corrupt_index',
  sidecar: {
    state: 'present',
    writtenAt: 1_699_996_400,
    generation: 7,
    counts: COUNTS,
    reason: null,
  },
  rebuildFailed: null,
  gapCountsRecoverable: false,
};

const ABSENT: SidecarReport = {
  state: 'absent',
  writtenAt: null,
  generation: null,
  counts: null,
  reason: null,
};
const ABSENT_FACT: CorruptFact = { ...corrupt, sidecar: ABSENT };
const UNREADABLE_FACT: CorruptFact = {
  ...corrupt,
  sidecar: { ...ABSENT, state: 'unreadable', reason: 'checksum mismatch' },
};
const NEWER_FACT: CorruptFact = {
  ...corrupt,
  sidecar: { ...ABSENT, state: 'newer', reason: 'schema 99 is newer than 21' },
};
const OTHER_STATES: readonly CorruptFact[] = [ABSENT_FACT, UNREADABLE_FACT, NEWER_FACT];

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

  // §48.7.2: nothing has been moved before REBUILD, whatever the sidecar beside it is.
  it('claims no file was set aside, in any sidecar state', () => {
    for (const fact of [corrupt, ...OTHER_STATES, { ...corrupt, rebuildFailed: 'planted' }]) {
      const copy = failureCopy(fact);
      const blocks = corruptLedger(fact).flatMap((block) => [block.label, ...block.lines]);
      expect([copy.headline, ...copy.body, ...blocks].join(' ')).not.toContain('set aside');
    }
  });

  // §48.7.2: the sidecar's counts are what a rebuild will restore; what the scan re-derives is
  // unknown until it runs, so that block is named and never counted.
  it('counts the sidecar in the restored block, each under its noun, and never the re-derived one', () => {
    const [reDerived, restored] = corruptLedger(corrupt);
    expect(reDerived?.lines.join(' ')).not.toMatch(/\d/);
    const text = restored?.lines.join(' ') ?? '';
    const listed = Object.entries(SIDECAR_COUNT_NOUNS).filter(([key]) => key !== 'merges');
    expect(listed.length).toBeGreaterThan(0);
    for (const [key, [one, many]] of listed) {
      const n = COUNTS[key] ?? 0;
      expect(text, key).toContain(`${String(n)} ${n === 1 ? one : many}`);
    }
  });

  // §48.8.3: merges are exported and never replayed, so a rebuild does not restore them and
  // the block may not list them as restored.
  it('does not list the merges a rebuild never replays', () => {
    const noun = SIDECAR_COUNT_NOUNS['merges'];
    const n = COUNTS['merges'];
    expect(noun).toBeDefined();
    expect(n).toBeGreaterThan(1);
    const text = corruptLedger(corrupt)[1]?.lines.join(' ') ?? '';
    expect(text).not.toContain(`${String(n)} ${noun?.[1] ?? ''}`);
  });

  // §48.7.2: REBUILD is offered while a rebuild can restore or start over, and a sidecar a newer
  // build wrote refuses it, so that window offers QUIT alone.
  it('offers REBUILD for a present, absent or unreadable sidecar and QUIT alone for a newer one', () => {
    for (const fact of [corrupt, ABSENT_FACT, UNREADABLE_FACT]) {
      expect(offersRebuild(fact), fact.sidecar.state).toBe(true);
      expect(failureCopy(fact).primary).toBe('REBUILD');
      expect(failureCopy(fact).secondary).toBe('QUIT');
    }
    expect(offersRebuild(NEWER_FACT)).toBe(false);
    expect(failureCopy(NEWER_FACT).primary).toBe('QUIT');
    expect(failureCopy(NEWER_FACT).secondary).toBeNull();
    expect(offersRebuild({ kind: 'schema_from_future', onDisk: 9, supported: 5 })).toBe(false);
  });

  // §48.7.2: with nothing to restore from, the window says what REBUILD will do in its own
  // words, and the restored block prints no figure.
  it('says something different for each sidecar state, and restores no figure without one', () => {
    const lead = (fact: CorruptFact): string => failureCopy(fact).body[0] ?? '';
    for (const fact of OTHER_STATES) {
      expect(lead(fact), fact.sidecar.state).not.toBe(lead(corrupt));
      const restored = corruptLedger(fact)[1]?.lines.join(' ') ?? '';
      expect(restored.length).toBeGreaterThan(0);
      expect(restored).not.toMatch(/\d/);
    }
    expect(lead(NEWER_FACT)).not.toBe(lead(ABSENT_FACT));
  });

  // A REBUILD that failed says so, once, with the core's reason.
  it('adds one line naming a failed rebuild', () => {
    const failed = { ...corrupt, rebuildFailed: 'the rebuild failed: planted' };
    const body = failureCopy(failed).body;
    expect(body).toHaveLength(failureCopy(corrupt).body.length + 1);
    expect(body.filter((line) => line.includes('the rebuild failed: planted'))).toHaveLength(1);
  });

  it('gives the gap its start time', () => {
    const gap = corruptLedger(corrupt)[2];
    expect(gap?.lines.join(' ')).toMatch(/\d{4}/);
  });

  it('says the gap start is unknown rather than printing a zero or a date', () => {
    const gap = corruptLedger({ ...corrupt, sidecar: { ...corrupt.sidecar, writtenAt: null } })[2];
    const text = gap?.lines.join(' ') ?? '';
    expect(text).toContain('not known');
    expect(text).not.toMatch(/1970|\b0\b/);
  });

  it('prints no figure at all when the gap cannot be counted', () => {
    const gap = corruptLedger(corrupt)[2];
    const text = gap?.lines.join(' ') ?? '';
    expect(text).toContain('cannot be counted');
    expect(text).not.toMatch(/\b\d+\s+(notes|sessions|projects)\b/);
  });

  // `0 projects` is a claim about what was lost, made on the one screen where unknown-as-zero
  // costs the most. What the scan re-derives is unknown in every state, and says so.
  it('prints no figure for a block the report could not count, and says so', () => {
    for (const fact of [corrupt, ...OTHER_STATES]) {
      const text = corruptLedger(fact)[0]?.lines.join(' ') ?? '';
      expect(text, fact.sidecar.state).toContain('not known');
      expect(text).not.toMatch(/\d/);
    }
  });

  it('states a measured empty ledger as nothing, and still prints no zeroed nouns', () => {
    const empty = Object.fromEntries(Object.keys(COUNTS).map((key) => [key, 0]));
    const sidecar = { ...corrupt.sidecar, counts: empty };
    const text = corruptLedger({ ...corrupt, sidecar })[1]?.lines.join(' ') ?? '';
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

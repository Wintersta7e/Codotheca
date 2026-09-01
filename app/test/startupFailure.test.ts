import { describe, expect, it } from 'vitest';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  EXIT_INDEX_FATAL,
  STARTUP_FAILURE_FILE,
  clearStartupFailure,
  readStartupFailure,
} from '../src/main/startupFailure';

function dir(): string {
  return mkdtempSync(join(tmpdir(), 'codotheca-'));
}

describe('the fatal-index report', () => {
  it('reads a report the core wrote', () => {
    const d = dir();
    writeFileSync(
      join(d, STARTUP_FAILURE_FILE),
      JSON.stringify({ kind: 'schema_from_future', onDisk: 9, supported: 5 }),
    );
    const failure = readStartupFailure(d);
    expect(failure).toEqual({ kind: 'schema_from_future', onDisk: 9, supported: 5 });
  });

  it('treats a missing, truncated or unknown report as no failure at all', () => {
    const d = dir();
    expect(readStartupFailure(d)).toBeNull();
    writeFileSync(join(d, STARTUP_FAILURE_FILE), '{ not json');
    expect(readStartupFailure(d)).toBeNull();
    writeFileSync(join(d, STARTUP_FAILURE_FILE), JSON.stringify({ kind: 'meteor' }));
    expect(readStartupFailure(d)).toBeNull();
  });

  it('reads a corrupt-index report whose ledger has no figures yet', () => {
    const d = dir();
    writeFileSync(
      join(d, STARTUP_FAILURE_FILE),
      JSON.stringify({
        kind: 'corrupt_index',
        quarantinedAt: 900,
        gapStartedAt: null,
        gapCountsRecoverable: false,
        reDerivable: null,
        restorable: null,
      }),
    );
    const failure = readStartupFailure(d);
    // `null` is "no rebuild has run", which the window renders as no figure at all. An empty
    // LedgerCounts here would print `0 projects restorable`, which is a claim, not an absence.
    expect(failure).toMatchObject({ kind: 'corrupt_index', reDerivable: null, restorable: null });
  });

  it('clears without throwing when there is nothing to clear', () => {
    const d = dir();
    expect(() => clearStartupFailure(d)).not.toThrow();
  });
});

// R24: a value that lives in both languages is correct only while a test reads the other
// language's source. The shell decides it is looking at a fatal index by comparing the core's
// exit code, and it finds the report by name — so a drift in either makes the shell miss a
// failure the core did report and fall through to a generic crash message.
describe('the mirrored halves of the report', () => {
  const RUST = readFileSync(
    fileURLToPath(new URL('../../core/src/surfaces/startup_failure.rs', import.meta.url)),
    'utf8',
  );

  it('uses the exit code the core actually exits with', () => {
    const m = /pub const EXIT_INDEX_FATAL: u8 = (\d+);/.exec(RUST);
    expect(m, 'the core no longer declares EXIT_INDEX_FATAL in the expected form').not.toBeNull();
    expect(EXIT_INDEX_FATAL).toBe(Number(m?.[1]));
  });

  it('looks for the file the core actually writes', () => {
    const m = /pub const STARTUP_FAILURE_FILE: &str = "([^"]+)";/.exec(RUST);
    expect(
      m,
      'the core no longer declares STARTUP_FAILURE_FILE in the expected form',
    ).not.toBeNull();
    expect(STARTUP_FAILURE_FILE).toBe(m?.[1]);
  });

  it('switches on the three tags the core can serialise', () => {
    // `#[serde(tag = "kind", rename_all = "snake_case")]` turns each variant name into these.
    for (const variant of ['SchemaFromFuture', 'MigrationFailed', 'CorruptIndex']) {
      expect(RUST).toContain(`    ${variant} {`);
    }
  });
});

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
import {
  CORRUPT_INDEX_FIELDS,
  SIDECAR_REPORT_FIELDS,
  SIDECAR_STATES,
} from '../src/shared/startupFailure';

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

  // The bytes the release core writes for a corrupt index with no sidecar beside it.
  const CORRUPT = {
    kind: 'corrupt_index',
    sidecar: { state: 'absent', writtenAt: null, generation: null, counts: null, reason: null },
    rebuildFailed: null,
    gapCountsRecoverable: false,
  };

  it('reads the corrupt-index report the core writes', () => {
    const d = dir();
    writeFileSync(join(d, STARTUP_FAILURE_FILE), JSON.stringify(CORRUPT));
    expect(readStartupFailure(d)).toEqual(CORRUPT);
  });

  // §48.7.1: the report before a rebuild names the sidecar's state and no quarantine. A report
  // in any other shape is from another build, and a window drawn from it would read fields that
  // are not there.
  it("refuses a corrupt-index report whose keys are not exactly the core's", () => {
    const d = dir();
    const refused = [
      {
        kind: 'corrupt_index',
        quarantinedAt: 900,
        gapStartedAt: null,
        gapCountsRecoverable: false,
        reDerivable: null,
        restorable: null,
      },
      { ...CORRUPT, quarantinedAt: 900 },
      { kind: 'corrupt_index', sidecar: CORRUPT.sidecar, rebuildFailed: null },
      { ...CORRUPT, sidecar: { ...CORRUPT.sidecar, quarantined: true } },
      { ...CORRUPT, sidecar: { state: 'absent' } },
      { ...CORRUPT, sidecar: { ...CORRUPT.sidecar, state: 'moved' } },
      { ...CORRUPT, sidecar: null },
    ];
    for (const report of refused) {
      writeFileSync(join(d, STARTUP_FAILURE_FILE), JSON.stringify(report));
      expect(readStartupFailure(d), JSON.stringify(report)).toBeNull();
    }
  });

  it('clears without throwing when there is nothing to clear', () => {
    const d = dir();
    expect(() => {
      clearStartupFailure(d);
    }).not.toThrow();
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

  // §48.13's mirror: the reader refuses any corrupt-index report whose keys differ from these
  // constants, so a field the core gains or loses would make every report unreadable and the
  // window would never be drawn.
  it('names every corrupt-index field, sidecar field and sidecar state the core serialises', () => {
    const block = (open: string, close: string): string => {
      const at = RUST.indexOf(open);
      expect(at, `the core no longer declares \`${open.trim()}\``).toBeGreaterThanOrEqual(0);
      return RUST.slice(at + open.length, RUST.indexOf(close, at));
    };
    // `rename_all = "camelCase"` on both, `rename_all = "snake_case"` on the state.
    const camel = (name: string): string =>
      name.replace(/_([a-z])/gu, (_m, c: string) => c.toUpperCase());
    const snake = (name: string): string => name.replace(/([a-z])([A-Z])/gu, '$1_$2').toLowerCase();
    const fields = (body: string): string[] =>
      [...body.matchAll(/^\s+(?:pub )?([a-z][a-z0-9_]*):/gmu)].map((m) => camel(m[1] ?? ''));

    const corrupt = fields(block('    CorruptIndex {', '\n    },'));
    const sidecar = fields(block('pub struct SidecarReport {', '\n}'));
    const states = [
      ...block('pub enum SidecarReportState {', '\n}').matchAll(/^\s+([A-Z][A-Za-z0-9]*),$/gmu),
    ].map((m) => snake(m[1] ?? ''));
    // eslint-disable-next-line no-console -- the count compared is the evidence
    console.log(
      `startup report mirror: ${String(corrupt.length)} corrupt-index fields, ` +
        `${String(sidecar.length)} sidecar fields, ${String(states.length)} sidecar states`,
    );
    for (const read of [corrupt, sidecar, states]) expect(read.length).toBeGreaterThan(0);
    expect(corrupt).toEqual([...CORRUPT_INDEX_FIELDS]);
    expect(sidecar).toEqual([...SIDECAR_REPORT_FIELDS]);
    expect(states).toEqual([...SIDECAR_STATES]);
  });

  it('switches on the three tags the core can serialise', () => {
    // `#[serde(tag = "kind", rename_all = "snake_case")]` turns each variant name into these.
    for (const variant of ['SchemaFromFuture', 'MigrationFailed', 'CorruptIndex']) {
      expect(RUST).toContain(`    ${variant} {`);
    }
  });
});

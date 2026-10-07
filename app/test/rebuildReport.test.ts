import { describe, expect, it } from 'vitest';
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { ackRebuildReport, readRebuildReport } from '../src/main/rebuildReport';
import { REBUILD_REPORT_FIELDS, REBUILD_REPORT_FILE } from '../src/shared/rebuildReport';

function dir(): string {
  return mkdtempSync(join(tmpdir(), 'codotheca-'));
}

// The bytes a rebuild that restored from a sidecar writes.
const REPORT = {
  quarantinedAt: 1_787_126_520,
  quarantineFiles: ['<data>/index.db.corrupt-1787126520', '<data>/sidecar.json.corrupt-1787126520'],
  restored: { projects: 3, notes: 2, roots: 0 },
  pending: 4,
  gapStartedAt: 1_787_000_000,
};

describe('the rebuild report', () => {
  it('reads the report the core wrote', () => {
    const d = dir();
    writeFileSync(join(d, REBUILD_REPORT_FILE), JSON.stringify(REPORT));
    expect(readRebuildReport(d)).toEqual(REPORT);
    // With no sidecar the gap has no start.
    writeFileSync(join(d, REBUILD_REPORT_FILE), JSON.stringify({ ...REPORT, gapStartedAt: null }));
    expect(readRebuildReport(d)?.gapStartedAt).toBeNull();
  });

  it('treats a missing, truncated or unrecognised report as none', () => {
    const d = dir();
    expect(readRebuildReport(d)).toBeNull();
    const refused: unknown[] = [
      '{ not json',
      null,
      [],
      { ...REPORT, extra: 1 },
      { ...REPORT, pending: undefined },
      { ...REPORT, pending: '4' },
      { ...REPORT, quarantineFiles: [7] },
      { ...REPORT, restored: { projects: 'three' } },
      { ...REPORT, restored: null },
      { ...REPORT, gapStartedAt: 'yesterday' },
    ];
    for (const report of refused) {
      const text = typeof report === 'string' ? report : JSON.stringify(report);
      writeFileSync(join(d, REBUILD_REPORT_FILE), text);
      expect(readRebuildReport(d), text).toBeNull();
    }
  });

  // §48.7.1 step 5: the acknowledgement is the only dismissal, and it removes the file.
  it('removes the report on acknowledgement, and acknowledging twice does not throw', () => {
    const d = dir();
    writeFileSync(join(d, REBUILD_REPORT_FILE), JSON.stringify(REPORT));
    ackRebuildReport(d);
    expect(existsSync(join(d, REBUILD_REPORT_FILE))).toBe(false);
    expect(readRebuildReport(d)).toBeNull();
    expect(() => {
      ackRebuildReport(d);
    }).not.toThrow();
  });
});

// A value that lives in both languages is correct only while a test reads the other language's
// source: the reader refuses a report whose keys are not exactly these, so a field the core gains
// would make every report unreadable and the notice would never be raised.
describe('the mirrored halves of the rebuild report', () => {
  const RUST = readFileSync(
    fileURLToPath(new URL('../../core/src/index/rebuild.rs', import.meta.url)),
    'utf8',
  );

  it('looks for the file the core actually writes', () => {
    const m = /pub const REBUILD_REPORT_FILE: &str = "([^"]+)";/.exec(RUST);
    expect(
      m,
      'the core no longer declares REBUILD_REPORT_FILE in the expected form',
    ).not.toBeNull();
    expect(REBUILD_REPORT_FILE).toBe(m?.[1]);
  });

  it('names every field the core serialises', () => {
    const open = 'pub struct RebuildReportFile {';
    const at = RUST.indexOf(open);
    expect(at, 'the core no longer declares `RebuildReportFile`').toBeGreaterThanOrEqual(0);
    const body = RUST.slice(at + open.length, RUST.indexOf('\n}', at));
    // `rename_all = "camelCase"` on the struct.
    const camel = (name: string): string =>
      name.replace(/_([a-z])/gu, (_m, c: string) => c.toUpperCase());
    const fields = [...body.matchAll(/^\s+pub ([a-z][a-z0-9_]*):/gmu)].map((m) =>
      camel(m[1] ?? ''),
    );
    // eslint-disable-next-line no-console -- the count compared is the evidence
    console.log(`rebuild report mirror: ${String(fields.length)} fields`);
    expect(fields.length).toBeGreaterThan(0);
    expect(fields).toEqual([...REBUILD_REPORT_FIELDS]);
  });
});

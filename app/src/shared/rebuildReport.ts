/**
 * §48.7.1 step 5's report as pure data, in `shared` so both processes can name it.
 *
 * A rebuild that swaps a fresh index into place writes this file beside it. The shell reads it,
 * because reading is a filesystem act, and removes it when the user acknowledges the notice that
 * states it — the acknowledgement is the only dismissal, so nothing records one in `view_state`.
 */

/** Must equal `REBUILD_REPORT_FILE` in `core/src/index/rebuild.rs`; a test reads it there. */
export const REBUILD_REPORT_FILE = 'rebuild-report.json';

export interface RebuildReport {
  /** When the quarantine happened, in Unix seconds; every set-aside name carries it. */
  readonly quarantinedAt: number;
  /** Where each set-aside file is now, as the core displayed the path. */
  readonly quarantineFiles: readonly string[];
  /** How many of each record came back, by the sidecar's count keys. */
  readonly restored: Readonly<Record<string, number>>;
  /** Records that wait for a scan to find their subject. */
  readonly pending: number;
  /** The sidecar's write time: nothing made after it came back. `null` with no sidecar. */
  readonly gapStartedAt: number | null;
}

/** `RebuildReport`'s fields, as the core serialises them. */
export const REBUILD_REPORT_FIELDS = [
  'quarantinedAt',
  'quarantineFiles',
  'restored',
  'pending',
  'gapStartedAt',
] as const;

/**
 * Whether `value` is a report in this build's shape. A report with any other keys is another
 * build's, and a notice drawn from it would state fields that are not there.
 */
export function isRebuildReport(value: unknown): value is RebuildReport {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const keys = Object.keys(value).sort();
  const want = [...REBUILD_REPORT_FIELDS].sort();
  if (keys.length !== want.length || keys.some((key, i) => key !== want[i])) return false;
  const report = value as Record<string, unknown>;
  const restored = report['restored'];
  const files = report['quarantineFiles'];
  const gap = report['gapStartedAt'];
  return (
    Number.isInteger(report['quarantinedAt']) &&
    Array.isArray(files) &&
    files.every((file) => typeof file === 'string') &&
    typeof restored === 'object' &&
    restored !== null &&
    !Array.isArray(restored) &&
    Object.values(restored).every((n) => Number.isInteger(n)) &&
    Number.isInteger(report['pending']) &&
    (gap === null || Number.isInteger(gap))
  );
}

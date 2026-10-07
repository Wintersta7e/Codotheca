// The shell half of §48.7.1 step 5. Like the startup-failure report, the core leaves one small
// JSON file beside the index; this reads it, and removes it only when the user acknowledges it.
import { readFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { REBUILD_REPORT_FILE, isRebuildReport, type RebuildReport } from '../shared/rebuildReport';

/** Never throws. A missing, truncated or unrecognised report is no report at all. */
export function readRebuildReport(dataDir: string): RebuildReport | null {
  let text: string;
  try {
    text = readFileSync(join(dataDir, REBUILD_REPORT_FILE), 'utf8');
  } catch {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return null;
  }
  return isRebuildReport(parsed) ? parsed : null;
}

/** The user's acknowledgement: the notice's only dismissal. Nothing to remove is not an error. */
export function ackRebuildReport(dataDir: string): void {
  rmSync(join(dataDir, REBUILD_REPORT_FILE), { force: true });
}

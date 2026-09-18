/**
 * [p3] §32.12's gate: the renderer may never originate an OS notification.
 *
 * Hand-written beside the `.mjs` for the same reason `scripts/lib/read-scanned.d.mts` is: the app
 * tsconfigs set no `allowJs`, so a test importing a gate needs the shape declared rather than
 * inferred.
 */
export interface NotificationOriginViolation {
  /** Repo-relative, so a failure names a path a reader can open. */
  file: string;
  /** Which spelling of the API was found. */
  token: string;
}

export interface NotificationOriginReport {
  /** **Printed and asserted**: a gate whose passing run scans zero files is a failing gate. */
  filesScanned: number;
  violations: NotificationOriginViolation[];
}

export function scanNotificationOrigins(root?: string): NotificationOriginReport;

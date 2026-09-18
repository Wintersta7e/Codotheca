// [p3] §32.12: **the renderer may never originate an OS notification.**
//
// The same invariant as *the renderer may never originate a filesystem path or an executable*,
// applied to the one outward-facing interruption the product has. The shell posts; the renderer's
// `notifications` permission stays denied, and `app/src/main/security.ts` is what denies it — this
// scanner is what stops a renderer file reaching for the API and finding the denial only at
// runtime, on a user's machine, where the failure is silence.
//
// **A gate whose passing run scans zero files is a failing gate**, so the count is printed and a
// zero exits non-zero.
import { readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

import { readScannedFile } from './lib/read-scanned.mjs';

const ROOT = fileURLToPath(new URL('..', import.meta.url));
const RENDERER = join(ROOT, 'app', 'src', 'renderer');
const EXTENSIONS = ['.ts', '.tsx', '.js', '.jsx'];

/** What originating an OS notification looks like, in every spelling the web platform offers. */
const FORBIDDEN = [
  'new Notification(',
  'window.Notification',
  'Notification.requestPermission',
  'self.registration.showNotification',
];

function walk(dir, out) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  for (const entry of entries) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(path, out);
    } else if (EXTENSIONS.some((ext) => entry.name.endsWith(ext))) {
      out.push(path);
    }
  }
  return out;
}

export function scanNotificationOrigins(root = RENDERER) {
  const violations = [];
  let filesScanned = 0;
  for (const path of walk(root, [])) {
    const text = readScannedFile(path);
    // A file that vanished between the walk and the read carries nothing to check and is **not
    // counted** — counting it would make the "scanned nothing" guard stop meaning what it says.
    if (text === null) continue;
    filesScanned += 1;
    for (const token of FORBIDDEN) {
      if (text.includes(token)) {
        violations.push({ file: relative(ROOT, path), token });
      }
    }
  }
  return { filesScanned, violations };
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const report = scanNotificationOrigins();
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  if (report.filesScanned === 0) {
    process.stderr.write('check-notification-origin: scanned zero files\n');
    process.exit(2);
  }
  if (report.violations.length > 0) {
    process.stderr.write(
      `check-notification-origin: ${String(report.violations.length)} violation(s)\n`,
    );
    process.exit(1);
  }
}

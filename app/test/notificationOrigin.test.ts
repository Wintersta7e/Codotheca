/**
 * [p3] **AC-P3-32-22** — the renderer may never originate an OS notification, and the gate that
 * says so must be able to fail.
 *
 * A gate whose passing run scans zero files is a failing gate, so the count is asserted as well as
 * the verdict; and the probe below proves the scanner bites rather than merely reporting clean.
 */
import { mkdtempSync, rmSync, writeFileSync, mkdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import { scanNotificationOrigins } from '../../scripts/check-notification-origin.mjs';

describe('no renderer file originates an OS notification', () => {
  it('AC-P3-32-22 scans the real tree, reports a non-zero count and finds nothing', () => {
    const report = scanNotificationOrigins();
    expect(report.filesScanned).toBeGreaterThan(0);
    expect(report.violations).toEqual([]);
  });

  it('AC-P3-32-22 reports a violation for every spelling the platform offers', () => {
    // A directory of its own rather than a probe inside `app/src/renderer`: vitest runs the node
    // project's files in parallel, and a probe planted in a tree other gates are walking is read
    // by one of them mid-life.
    const dir = mkdtempSync(join(tmpdir(), 'codotheca-notify-'));
    try {
      mkdirSync(join(dir, 'nested'));
      writeFileSync(join(dir, 'a.ts'), "new Notification('x');\n");
      writeFileSync(join(dir, 'nested', 'b.tsx'), 'window.Notification;\n');
      writeFileSync(join(dir, 'clean.ts'), 'export const ok = 1;\n');
      const report = scanNotificationOrigins(dir);
      expect(report.filesScanned).toBe(3);
      expect(report.violations.map((v) => v.token).sort()).toEqual([
        'new Notification(',
        'window.Notification',
      ]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it('a scan over an empty tree reports zero, which the CLI treats as a failure', () => {
    const dir = mkdtempSync(join(tmpdir(), 'codotheca-notify-empty-'));
    try {
      expect(scanNotificationOrigins(dir).filesScanned).toBe(0);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});

#!/usr/bin/env node
/**
 * A skipped end-to-end spec is a failing gate, on every platform.
 *
 * `mount.spec.ts` and `scan-shelf.spec.ts` both call `test.skip()` when the release core is
 * absent, so the suite degrades to green **silently** — CI's first run passed in 41 seconds
 * having asserted nothing about a painted screen. The check that caught that was
 * `grep -qiE '[0-9]+ skipped'` in a bash step, which is why the e2e job could only ever run on
 * Linux; this reads Playwright's own report instead and runs anywhere Node does.
 *
 * It also prints the count it scanned: a gate whose passing run examined zero specs is a gate
 * that cannot fail, and printing the number is what makes that visible (§26.2).
 *
 * Usage: node scripts/check-e2e-skips.mjs [report.json]
 */
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

// `fileURLToPath`, never `.pathname`: a file URL's pathname keeps a leading slash, and on
// Windows the drive letter sits after it, so `readFileSync` opens a doubled-drive path.
const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const reportPath = process.argv[2] ?? join(root, 'app', 'e2e-report.json');

if (!existsSync(reportPath)) {
  console.error(`check-e2e-skips: no report at ${reportPath} — the suite did not run.`);
  process.exit(2);
}

/** @type {{suites?: unknown[]}} */
const report = JSON.parse(readFileSync(reportPath, 'utf8'));

const specs = [];
/** @param {{specs?: unknown[], suites?: unknown[], title?: string}} suite */
function walk(suite, trail) {
  const here = suite.title === undefined ? trail : [...trail, suite.title];
  for (const spec of suite.specs ?? []) {
    const runs = (spec.tests ?? []).flatMap((t) => t.results ?? []);
    const skipped =
      (spec.tests ?? []).length === 0 ||
      (runs.length > 0 && runs.every((r) => r.status === 'skipped'));
    specs.push({ title: [...here, spec.title].join(' › '), skipped, ok: spec.ok === true });
  }
  for (const child of suite.suites ?? []) walk(child, here);
}
for (const suite of report.suites ?? []) walk(suite, []);

const skipped = specs.filter((s) => s.skipped);

if (specs.length === 0) {
  console.error('check-e2e-skips: the report holds no specs at all — nothing was asserted.');
  process.exit(1);
}

if (skipped.length > 0) {
  for (const spec of skipped) {
    console.error(`::error::e2e spec skipped, so it asserted nothing: ${spec.title}`);
  }
  console.error(
    'check-e2e-skips: a missing release core is the usual cause — it is the precondition ' +
      'both mount.spec.ts and scan-shelf.spec.ts skip on.',
  );
  process.exit(1);
}

console.log(`check-e2e-skips: ok — ${String(specs.length)} spec(s), 0 skipped`);

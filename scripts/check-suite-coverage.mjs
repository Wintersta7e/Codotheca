#!/usr/bin/env node
/**
 * A test file that vanishes from the report asserted nothing, and no other gate says so.
 *
 * Measured on `main`: one full run of three exited 1 carrying `numFailedTests: 0` and a total of
 * 2372 rather than 2375, because `app/src/renderer/palette/motion.test.ts` was absent from
 * `testResults` altogether — no assertion failure, no error line, no pool message. Only the exit
 * code differed from a green run, and `scripts/acceptance.mjs` reads assertion results and never
 * reads `numTotalTests`, so the capture it stores is shaped exactly like success. Two different
 * faults have been sharing one name because of it: a worker that never starts loses several files
 * at once and prints a pool error, while a file that vanishes silently prints nothing at all.
 *
 * **The denominator is vitest's own collection, never a literal.** An expected-count constant is
 * the R22/R24 class — every lane that adds a test edits the number, and the number is what drifts.
 * `vitest list` re-globs on every run, so the count maintains itself and this gate has no second
 * owner to fall out of step with.
 *
 * Like the other gates here it prints what it scanned: a passing run that examined zero files is
 * a gate that cannot fail (§26.2).
 *
 * Usage: node scripts/check-suite-coverage.mjs [report.json] [collected.json]
 *
 * `collected.json` is for this gate's own tests. Without it, vitest is asked to collect, which is
 * the point — the glob is the bar.
 */
import { spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

// `fileURLToPath`, never `.pathname`: a file URL's pathname keeps a leading slash, and on Windows
// the drive letter sits after it, so `readFileSync` opens a doubled-drive path.
const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const appDir = join(root, 'app');
const reportPath = process.argv[2] ?? join(root, 'acceptance', 'results', 'vitest.json');
const collectedArg = process.argv[3];

/** One spelling for a path, so a separator difference is never read as a missing file. */
function key(file) {
  return resolve(file).split(sep).join('/');
}

/**
 * Ask vitest to collect without running. Resolved from its own `bin` field and run through *this*
 * Node, the way `package.mjs` reaches electron-builder: spawning `npx` would need a shell on
 * Windows, and a WSL-installed `.bin` holds symlinks `cmd.exe` cannot execute.
 */
function collectFromVitest() {
  const require_ = createRequire(join(appDir, 'package.json'));
  const pkgPath = require_.resolve('vitest/package.json');
  const bin = join(dirname(pkgPath), JSON.parse(readFileSync(pkgPath, 'utf8')).bin.vitest);
  const dir = mkdtempSync(join(tmpdir(), 'codotheca-suite-'));
  const out = join(dir, 'collected.json');
  try {
    const run = spawnSync(process.execPath, [bin, 'list', `--json=${out}`], {
      cwd: appDir,
      encoding: 'utf8',
    });
    if (run.status !== 0 || !existsSync(out)) {
      console.error(
        `check-suite-coverage: vitest could not collect (exit ${String(run.status)}). ` +
          'Without the glob there is no denominator, so this is a failure, not a skip.',
      );
      console.error(run.stderr ?? '');
      process.exit(2);
    }
    return JSON.parse(readFileSync(out, 'utf8'));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (!existsSync(reportPath)) {
  console.error(
    `check-suite-coverage: no report at ${reportPath} — the suite did not run, so there is ` +
      'nothing to check for gaps.',
  );
  process.exit(2);
}

const collected =
  collectedArg === undefined ? collectFromVitest() : JSON.parse(readFileSync(collectedArg, 'utf8'));

/** Every file vitest's glob reached that holds at least one test. */
const expected = new Set(collected.map((entry) => key(entry.file)));

/**
 * Every file the report accounts for. A file present with an empty `assertionResults` is as much
 * a loss as an absent one — it occupies a row and asserts nothing.
 */
const report = JSON.parse(readFileSync(reportPath, 'utf8'));
const reported = new Set(
  (report.testResults ?? [])
    .filter((file) => (file.assertionResults ?? []).length > 0)
    .map((file) => key(file.name)),
);

if (expected.size === 0) {
  console.error('check-suite-coverage: vitest collected no test files at all — the glob is empty.');
  process.exit(1);
}

const missing = [...expected].filter((file) => !reported.has(file)).sort();

if (missing.length > 0) {
  for (const file of missing) {
    console.error(`::error::test file collected but absent from the report: ${file}`);
  }
  console.error(
    `check-suite-coverage: ${String(missing.length)} of ${String(expected.size)} collected file(s) ` +
      'asserted nothing. A run that loses a file reports zero failures, so the count is the only ' +
      'evidence — re-run and compare identity, not totals.',
  );
  process.exit(1);
}

console.log(
  `check-suite-coverage: ok — ${String(expected.size)} collected file(s), all present in the report`,
);

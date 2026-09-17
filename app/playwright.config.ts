import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: 'e2e',
  // Electron start-up dominates; one worker, because the app holds a single-instance lock and
  // a second copy would focus the first rather than opening a window of its own.
  workers: 1,
  timeout: 60_000,
  // `list` for a person, `json` because `scripts/check-e2e-skips.mjs` reads Playwright's own
  // report rather than grepping console output. Emitting it locally is not optional: with
  // `list` alone the local run wrote no report, the checker read a gitignored one from a
  // previous day, and it called two specs skipped that had just passed. CI overrides this with
  // `--reporter=list,json` and `PLAYWRIGHT_JSON_OUTPUT_NAME`, which resolves to the same file.
  reporter: [['list'], ['json', { outputFile: 'e2e-report.json' }]],
});

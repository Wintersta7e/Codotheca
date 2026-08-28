import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: 'e2e',
  // Electron start-up dominates; one worker, because the app holds a single-instance lock and
  // a second copy would focus the first rather than opening a window of its own.
  workers: 1,
  timeout: 60_000,
  reporter: 'list',
});

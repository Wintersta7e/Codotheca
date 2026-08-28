import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

// Two projects rather than one environment. Renderer code needs a DOM; main- and preload-side
// code must not silently get one, or a test can pass against a global the sandboxed shell has
// never had. Vitest 3 removed `environmentMatchGlobs`, which is how the plan expressed this,
// and `projects` is its replacement.
export default defineConfig({
  test: {
    projects: [
      {
        plugins: [react()],
        test: {
          name: 'node',
          include: [
            'src/main/**/*.test.ts',
            'src/preload/**/*.test.ts',
            'src/shared/**/*.test.ts',
            'test/**/*.test.ts',
          ],
          exclude: ['test/dom/**'],
          environment: 'node',
          restoreMocks: true,
          // The corpus test's beforeAll compiles and runs the Rust generator on a cold cache.
          testTimeout: 300_000,
        },
      },
      {
        plugins: [react()],
        test: {
          name: 'dom',
          include: [
            'src/renderer/**/*.test.ts',
            'src/renderer/**/*.test.tsx',
            'test/dom/**/*.test.ts',
          ],
          environment: 'jsdom',
          restoreMocks: true,
        },
      },
    ],
  },
});

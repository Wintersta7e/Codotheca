import * as path from 'node:path';
import { describe, expect, it } from 'vitest';
import { resolveWorkerBinary, WORKER_ARCHES, WORKER_BINARY_NAME } from './paths';

describe('resolveWorkerBinary', () => {
  for (const arch of WORKER_ARCHES) {
    it(`resolves the packaged ${arch} worker beside the core resources`, () => {
      expect(
        resolveWorkerBinary({
          isPackaged: true,
          resourcesPath: '/app/resources',
          appRoot: '/source',
          arch,
        }),
      ).toBe(path.join('/app/resources', 'worker', `linux-${arch}`, WORKER_BINARY_NAME));
    });

    it(`resolves the development ${arch} worker from its staged location`, () => {
      expect(
        resolveWorkerBinary({
          isPackaged: false,
          resourcesPath: '/app/resources',
          appRoot: '/source',
          arch,
        }),
      ).toBe(path.join('/source', 'build', 'worker', `linux-${arch}`, WORKER_BINARY_NAME));
    });
  }
});

import { act, renderHook, waitFor } from '@testing-library/react';
import { expect, it, vi } from 'vitest';

import { fakeAppDeps } from '../../src/renderer/app/testDeps';
import { useCoreStatus } from '../../src/renderer/app/useCoreStatus';
import { useRebuildReport } from '../../src/renderer/app/useRebuildReport';
import { createEventFanout } from '../../src/renderer/project/deps';
import type { RebuildReport } from '../../src/shared/rebuildReport';
import coreSnapshot from '../fixtures/coreSnapshot.json';

const REPORT: RebuildReport = {
  quarantinedAt: 1_700_000_200,
  quarantineFiles: ['<data>/index.db.corrupt-1700000200'],
  restored: { projects: 2 },
  pending: 0,
  gapStartedAt: null,
};

/**
 * The renderer half of the core snapshot's path. `src/main/core/bridge.test.ts` sends the core's
 * frame through the real client and bridge and asserts the batch it leaves as; that batch enters
 * here through the renderer's own fan-out, and both hooks that read the snapshot must see it.
 */
it('the batch the bridge sends for the core snapshot reaches both hooks that read it', async () => {
  const sinks: ((batch: unknown) => void)[] = [];
  let onDisk: RebuildReport | null = null;
  const rebuildReport = vi.fn(() => Promise.resolve(onDisk));
  const { deps } = fakeAppDeps(
    {},
    {
      subscribe: createEventFanout((cb) => {
        sinks.push(cb);
      }),
      rebuildReport,
    },
  );
  const status = renderHook(() => useCoreStatus(deps));
  const report = renderHook(() => useRebuildReport(deps));
  await waitFor(() => {
    expect(rebuildReport).toHaveBeenCalledTimes(1);
  });
  expect(sinks).toHaveLength(1);
  expect(status.result.current.firstRunCompletedAt).toBeNull();

  onDisk = REPORT;
  act(() => {
    for (const sink of sinks) sink(coreSnapshot.batch);
  });

  await waitFor(() => {
    expect(report.result.current.report).toEqual(REPORT);
  });
  expect(status.result.current.firstRunCompletedAt).toBe(1_700_000_000);
  expect(status.result.current.gitVersion).toBe('2.43.0');
});

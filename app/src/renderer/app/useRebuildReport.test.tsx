import { act, renderHook, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { RebuildReport } from '../../shared/rebuildReport';
import { fakeAppDeps } from './testDeps';
import { useRebuildReport } from './useRebuildReport';

const REPORT: RebuildReport = {
  quarantinedAt: 1_787_126_520,
  quarantineFiles: ['<data>/index.db.corrupt-1787126520'],
  restored: { projects: 3 },
  pending: 0,
  gapStartedAt: null,
};

describe('useRebuildReport', () => {
  it('reads the report when the window mounts', async () => {
    const { deps } = fakeAppDeps({}, { rebuildReport: () => Promise.resolve(REPORT) });
    const { result } = renderHook(() => useRebuildReport(deps));
    await waitFor(() => {
      expect(result.current.report).toEqual(REPORT);
    });
  });

  // The core greets before it rebuilds, so the report a rebuild writes can appear after the lane
  // is ready. The core's snapshot comes once its startup is over, and is read again then.
  it('reads again on the core snapshot, which follows the rebuild', async () => {
    let onDisk: RebuildReport | null = null;
    const rebuildReport = vi.fn(() => Promise.resolve(onDisk));
    const fake = fakeAppDeps({}, { rebuildReport });
    const { result } = renderHook(() => useRebuildReport(fake.deps));
    await waitFor(() => {
      expect(rebuildReport).toHaveBeenCalledTimes(1);
    });
    expect(result.current.report).toBeNull();

    onDisk = REPORT;
    act(() => {
      fake.emit({ topic: 'scan', event: 'run_started', data: {} });
    });
    expect(rebuildReport).toHaveBeenCalledTimes(1);
    act(() => {
      fake.emit({ topic: 'core', event: 'snapshot', data: {} });
    });
    await waitFor(() => {
      expect(result.current.report).toEqual(REPORT);
    });
    expect(rebuildReport).toHaveBeenCalledTimes(2);
  });

  // §48.7.1 step 5: the user's acknowledgement is the only dismissal, and it removes the file.
  it('acknowledging asks the shell to remove the report and stops raising it', async () => {
    const ackRebuildReport = vi.fn(() => Promise.resolve());
    const { deps } = fakeAppDeps(
      {},
      { rebuildReport: () => Promise.resolve(REPORT), ackRebuildReport },
    );
    const { result } = renderHook(() => useRebuildReport(deps));
    await waitFor(() => {
      expect(result.current.report).not.toBeNull();
    });
    act(() => {
      result.current.acknowledge();
    });
    expect(ackRebuildReport).toHaveBeenCalledTimes(1);
    expect(result.current.report).toBeNull();
  });
});

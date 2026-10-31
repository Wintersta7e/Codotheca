import { act, cleanup, render, renderHook, screen, waitFor } from '@testing-library/react';
import { createElement, type ReactElement } from 'react';
import { afterEach, expect, it, vi } from 'vitest';

import type { TargetId, TargetVerification } from '../../src/generated/protocol';
import type { AppDeps } from '../../src/renderer/app/deps';
import { fakeAppDeps } from '../../src/renderer/app/testDeps';
import { useNotices } from '../../src/renderer/app/useNotices';
import { useStaleTargets } from '../../src/renderer/app/useStaleTargets';
import { NoticeSlot } from '../../src/renderer/shelf/NoticeSlot';

afterEach(cleanup);

const missing: TargetVerification = {
  targetId: 7 as TargetId,
  verifyState: 'missing',
  verifiedAt: 1,
  execDisplay: 'editor.exe',
};

/** The window's own path from the shell's stale rows to the notice slot, as `App` wires it. */
function Slot(props: { readonly deps: AppDeps }): ReactElement | null {
  const staleTargets = useStaleTargets(props.deps);
  const notices = useNotices({
    degraded: null,
    gitVersion: null,
    spawnFailure: null,
    problems: null,
    identityToConfirm: false,
    sync: null,
    rebuildReport: null,
    staleTargets,
    onOpenLog: vi.fn(),
    onOpenScanSummary: vi.fn(),
    onAckRebuildReport: vi.fn(),
    onRevealIndex: vi.fn(),
  });
  return createElement(NoticeSlot, { candidates: notices, dismissed: [], onDismiss: vi.fn() });
}

it('AC-P4-50-2 a missing target raises the notice', async () => {
  // A window that mounts after the startup sweep reads what the shell kept.
  const { deps } = fakeAppDeps({}, { staleTargetsNow: () => Promise.resolve([missing]) });
  render(createElement(Slot, { deps }));
  const region = await screen.findByRole('region', { name: 'A LAUNCH TARGET NO LONGER RESOLVES' });
  expect(region.textContent).toContain('editor.exe');
});

it('AC-P4-50-2 a sweep that lands after the window mounted raises it too', async () => {
  const fake = fakeAppDeps();
  render(createElement(Slot, { deps: fake.deps }));
  expect(screen.queryByRole('region')).toBeNull();
  act(() => {
    fake.setStaleTargets([missing]);
  });
  expect(
    await screen.findByRole('region', { name: 'A LAUNCH TARGET NO LONGER RESOLVES' }),
  ).toBeTruthy();
});

it('AC-P4-50-2 unverified or not_executable raises none', async () => {
  // The shell's `staleTargets` keeps `missing` only, so such a sweep reaches the window empty.
  const staleTargetsNow = vi.fn(() => Promise.resolve([] as readonly TargetVerification[]));
  const { deps } = fakeAppDeps({}, { staleTargetsNow });
  render(createElement(Slot, { deps }));
  await waitFor(() => {
    expect(staleTargetsNow).toHaveBeenCalledTimes(1);
  });
  expect(screen.queryByRole('region')).toBeNull();
  const hook = renderHook(() => useStaleTargets(deps));
  await waitFor(() => {
    expect(hook.result.current).toEqual([]);
  });
});

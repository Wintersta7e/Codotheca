import { describe, expect, it, vi } from 'vitest';
import { logLevelStep, staleTargets, verifyTargetsStep } from '../src/main/joinSteps';
import type { TargetId, TargetVerification } from '../src/generated/protocol';

// `TargetId` is a branded number in the generated types, so a literal needs the cast; the
// brand is what stops a project id being passed where a target id belongs.
const id = (n: number): TargetId => n as TargetId;

const ok: TargetVerification = {
  targetId: id(1),
  verifyState: 'ok',
  verifiedAt: 1,
  execDisplay: 'editor',
};
const missing: TargetVerification = { ...ok, targetId: id(2), verifyState: 'missing' };
const unverified: TargetVerification = { ...ok, targetId: id(3), verifyState: 'unverified' };
const notExecutable: TargetVerification = {
  ...ok,
  targetId: id(4),
  verifyState: 'not_executable',
};

describe('the join steps', () => {
  it('verifies every target at startup and hands the rows on', async () => {
    const onResult = vi.fn();
    const request = vi.fn(() => Promise.resolve([ok, missing]));
    const step = verifyTargetsStep(request, onResult);
    expect(step.name).toBe('targets.verify');
    await step.run();
    // `{targetId: null}` is the all-rows sweep; §11.5's per-target window passes an id.
    expect(request).toHaveBeenCalledWith('targets.verify', { targetId: null });
    expect(onResult).toHaveBeenCalledWith([ok, missing]);
  });

  it('only missing rows are stale — unverified is not the same claim', () => {
    expect(staleTargets([ok, missing, unverified])).toEqual([missing]);
  });

  it('not_executable is a fact about a file that is there, so it is not stale either', () => {
    // §11.5 draws a different window for each; folding them would send the user looking for a
    // file that never moved.
    expect(staleTargets([notExecutable, unverified])).toEqual([]);
  });

  it('applies the stored log level to the rolling log', async () => {
    const setLevel = vi.fn();
    const request = vi.fn(() =>
      Promise.resolve({
        effectsTier: 'auto',
        reducedMotionOverride: false,
        autostart: false,
        residentShortcut: null,
        roastEnabled: true,
        logLevel: 'debug',
      }),
    );
    const step = logLevelStep(request, { setLevel });
    expect(step.name).toBe('log.level');
    await step.run();
    expect(request).toHaveBeenCalledWith('settings.get', {});
    expect(setLevel).toHaveBeenCalledWith('debug');
  });

  it('a failing step rejects rather than swallowing, so runStartup reports step_failed', async () => {
    const step = verifyTargetsStep(() => Promise.reject(new Error('core failed')), vi.fn());
    await expect(step.run()).rejects.toThrow('core failed');
  });
});

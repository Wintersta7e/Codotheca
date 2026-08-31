import { describe, expect, it, vi } from 'vitest';
import type { JoinStep } from '../startup';
import { launchJoinSteps, sessionRecoveryStep } from './launchSteps';

const noop = {
  onRecovered: () => undefined,
  onVerified: () => undefined,
};

describe('the launch lane join steps', () => {
  it('subscribes before it issues anything', async () => {
    // A session/ended emitted by orphan recovery must not be lost to a subscription that did
    // not exist yet.
    const order: string[] = [];
    const step = sessionRecoveryStep({
      request: () => {
        order.push('request');
        return Promise.resolve({});
      },
      subscribe: () => {
        order.push('subscribe');
        return () => undefined;
      },
      ...noop,
    });
    await step.run();
    expect(order).toEqual(['subscribe', 'request']);
  });

  it('establishes the join-time focus level, which is nothing', async () => {
    const request = vi.fn(() => Promise.resolve({}));
    await sessionRecoveryStep({
      request,
      subscribe: () => () => undefined,
      ...noop,
    }).run();
    expect(request).toHaveBeenCalledWith('session.focus', { projectId: null });
  });

  it('hands back the sessions recovery closed', async () => {
    const recovered: unknown[] = [];
    const step = sessionRecoveryStep({
      request: () => Promise.resolve({}),
      subscribe: (_topic, handler) => {
        handler.onEvent('ended', { session: { id: 4 } });
        handler.onEvent('ended', { session: { id: 5 } });
        handler.onEvent('started', { session: { id: 6 } });
        return () => undefined;
      },
      onRecovered: (rows) => recovered.push(...rows),
      onVerified: () => undefined,
    });
    await step.run();
    expect(recovered).toEqual([{ id: 4 }, { id: 5 }]);
  });

  it('unsubscribes when it is done, whether or not it succeeded', async () => {
    const unsubscribe = vi.fn();
    const failing = sessionRecoveryStep({
      request: () => Promise.reject(new Error('core failed')),
      subscribe: () => unsubscribe,
      ...noop,
    });
    await expect(failing.run()).rejects.toThrow('core failed');
    expect(unsubscribe).toHaveBeenCalledOnce();
  });

  it('runs recovery before verification, and declares no second verify step', () => {
    // §11.2: close orphaned sessions → join. §4bis.5: targets.verify runs at startup.
    // C6: verifyTargetsStep is plan 17's, so it is injected rather than redeclared here.
    const verify: JoinStep = { name: 'verify-targets', run: () => Promise.resolve() };
    const steps = launchJoinSteps(
      { request: () => Promise.resolve({}), subscribe: () => () => undefined, ...noop },
      verify,
    );
    expect(steps.map((s) => s.name)).toEqual(['session-recovery', 'verify-targets']);
  });

  it('omits the verify step entirely while plan 17 has not landed it', () => {
    // Better a lane that is honestly short one step than one that declares a duplicate of a
    // step another plan owns.
    const steps = launchJoinSteps({
      request: () => Promise.resolve({}),
      subscribe: () => () => undefined,
      ...noop,
    });
    expect(steps.map((s) => s.name)).toEqual(['session-recovery']);
  });

  it('names itself when it fails, so runStartup can report step_failed', async () => {
    const step = sessionRecoveryStep({
      request: () => Promise.reject(new Error('boom')),
      subscribe: () => () => undefined,
      ...noop,
    });
    expect(step.name).toBe('session-recovery');
    await expect(step.run()).rejects.toThrow('boom');
  });
});

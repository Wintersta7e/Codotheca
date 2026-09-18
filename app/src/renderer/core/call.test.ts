import { describe, expect, it, vi } from 'vitest';
import type { BridgeReply } from '../../shared/channels.js';
import { CallFailure, callWith, unwrapReply } from './call.js';

const settings = {
  effectsTier: 'auto',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: false,
  logLevel: 'info',
} as const;

describe('the renderer call wrapper', () => {
  it('returns the value of a successful reply and passes the args through unchanged', async () => {
    const request = vi.fn((): Promise<BridgeReply> =>
      Promise.resolve({ ok: true, value: settings }),
    );
    const call = callWith(request);
    const patch = {
      effectsTier: null,
      reducedMotionOverride: null,
      autostart: null,
      residentShortcut: null,
      roastEnabled: false,
      logLevel: null,
      installRootId: null,
      contentScanEnabled: null,
      healthChecks: null,
    };
    expect(await call('settings.set', { patch })).toEqual(settings);
    expect(request).toHaveBeenCalledWith('settings.set', { patch });
  });

  it('turns a failed reply into a typed error rather than a resolved undefined', async () => {
    const call = callWith((): Promise<BridgeReply> =>
      Promise.resolve({
        ok: false,
        error: {
          code: 'PERMISSION_DENIED',
          message: 'diagnostic',
          outcome: null,
          retryable: false,
        },
      }),
    );
    const failure = await call('settings.get', {}).then(
      () => null,
      (e: unknown) => e,
    );
    expect(failure).toBeInstanceOf(CallFailure);
    expect((failure as CallFailure).code).toBe('PERMISSION_DENIED');
    // §2.2: `null` is "definitely did not take effect", and it is a distinct answer from
    // `'unknown'`. A wrapper that folded both into one string would lose the distinction the
    // renderer needs to decide whether a retry is safe.
    expect((failure as CallFailure).outcome).toBeNull();
  });

  it('never puts the core diagnostic message in front of a user', async () => {
    const call = callWith((): Promise<BridgeReply> =>
      Promise.resolve({
        ok: false,
        error: {
          code: 'INTERNAL',
          message: 'sqlite: disk I/O error at <a real home directory>/x',
          outcome: 'unknown',
          retryable: true,
        },
      }),
    );
    const failure = (await call('settings.get', {}).catch((e: unknown) => e)) as CallFailure;
    // §2.4: the core's message is diagnostic and is never shown raw. It survives on `detail`
    // for the log; the string a surface renders comes from the shell's own prose.
    expect(failure.message).not.toContain('disk I/O');
    expect(failure.message).toBe('settings.get failed: INTERNAL');
    expect(failure.detail).toContain('disk I/O');
    expect(failure.retryable).toBe(true);
    expect(failure.outcome).toBe('unknown');
  });

  it('unwraps a shell channel reply with the same rule as a protocol reply', () => {
    expect(unwrapReply<number>({ ok: true, value: 7 })).toBe(7);
    expect(() =>
      unwrapReply({
        ok: false,
        error: { code: 'PATH_GONE', message: 'x', outcome: null, retryable: false },
      }),
    ).toThrow(CallFailure);
  });
});

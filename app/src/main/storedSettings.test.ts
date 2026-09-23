import { describe, expect, it, vi } from 'vitest';
import type { Settings } from '../generated/protocol.js';
import { onStoredSettings } from './storedSettings.js';

const answer = (residentShortcut: string | null): Settings => ({
  effectsTier: 'auto',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut,
  roastEnabled: true,
  logLevel: 'info',
  installRootId: null,
  contentScanEnabled: false,
  healthChecks: [],
});

describe('onStoredSettings', () => {
  it('re-applies the chord the drawer just wrote, so the row is not a dead switch', async () => {
    const applied: (string | null)[] = [];
    const request = onStoredSettings(
      (name) => Promise.resolve(name === 'settings.set' ? answer('Alt+F12') : answer(null)),
      (stored) => {
        applied.push(stored.residentShortcut);
      },
    );

    await request('settings.set', { patch: {} });
    expect(applied).toEqual(['Alt+F12']);
  });

  it('leaves every other command untouched', async () => {
    const applied: (string | null)[] = [];
    const request = onStoredSettings(
      () => Promise.resolve(answer('Alt+F12')),
      (stored) => {
        applied.push(stored.residentShortcut);
      },
    );
    await request('settings.get', {});
    expect(applied).toEqual([]);
  });

  it('hands one answer to every listener, in order, and returns it', async () => {
    const order: string[] = [];
    const request = onStoredSettings(
      () => Promise.resolve(answer(null)),
      () => order.push('first'),
      () => order.push('second'),
    );
    await expect(request('settings.set', { patch: {} })).resolves.toEqual(answer(null));
    expect(order).toEqual(['first', 'second']);
  });

  // An answer that is not a stored `Settings` stored nothing, so there is nothing to act on — not
  // an unbound chord, and not a TypeError for a listener to trip over.
  it('a null answer reaches no listener and still passes through', async () => {
    const listener = vi.fn();
    const request = onStoredSettings(() => Promise.resolve(null), listener);
    await expect(request('settings.set', { patch: {} })).resolves.toBeNull();
    expect(listener).not.toHaveBeenCalled();
  });
});

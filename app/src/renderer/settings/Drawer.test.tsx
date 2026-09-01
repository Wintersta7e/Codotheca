import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { CommandName, Settings } from '../../generated/protocol.js';
import {
  SETTINGS_ROWS,
  SettingsDrawer,
  toSettingsPatch,
  type SettingsDrawerDeps,
} from './Drawer.js';
import { SETTINGS_GROUP_ORDER } from './rows.js';

afterEach(cleanup);

const settings: Settings = {
  effectsTier: 'auto',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
};

const answers = (name: CommandName): unknown => {
  if (name === 'settings.get' || name === 'settings.set') return settings;
  if (name === 'targets.list') return { rows: [], resolved: null };
  if (name === 'identity.list') return [];
  if (name === 'roots.list') return [];
  return {};
};

const deps = (over: Partial<SettingsDrawerDeps> = {}): SettingsDrawerDeps => ({
  call: vi.fn((name: CommandName) => Promise.resolve(answers(name))) as never,
  shell: {
    pickRoot: vi.fn(() => Promise.resolve({ kind: 'cancelled' as const })),
    reveal: vi.fn(() => Promise.resolve({ ok: true, value: null })),
    indexLocation: vi.fn(() =>
      Promise.resolve({ ok: true, value: { pathDisplay: '<data>/index.db', sizeBytes: 2048 } }),
    ),
    onShortcutState: vi.fn(),
  },
  tier: 'full',
  ...over,
});

describe('the settings drawer', () => {
  it('renders nothing while closed', () => {
    const { container } = render(
      <SettingsDrawer open={false} onClose={vi.fn()} deps={deps()} slots={{}} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it('is a 400px dialog titled SETTINGS with an ESC chip', async () => {
    render(<SettingsDrawer open onClose={vi.fn()} deps={deps()} slots={{}} />);
    const dialog = await screen.findByRole('dialog', { name: 'SETTINGS' });
    expect(dialog.style.width).toBe('400px');
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(screen.getByText('ESC')).toBeTruthy();
  });

  it('Esc closes, and a click inside the panel does not', async () => {
    const onClose = vi.fn();
    render(<SettingsDrawer open onClose={onClose} deps={deps()} slots={{}} />);
    const dialog = await screen.findByRole('dialog', { name: 'SETTINGS' });
    fireEvent.click(dialog);
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.keyDown(dialog, { key: 'Escape', code: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('owns only the keys §11.7 gives the settings context', async () => {
    const onClose = vi.fn();
    render(<SettingsDrawer open onClose={onClose} deps={deps()} slots={{}} />);
    const dialog = await screen.findByRole('dialog', { name: 'SETTINGS' });
    // The one key table decides this, so a key the shelf owns does nothing here.
    for (const code of ['ArrowDown', 'Enter', 'Space', 'KeyP']) {
      fireEvent.keyDown(dialog, { key: code, code });
    }
    expect(onClose).not.toHaveBeenCalled();
  });

  it('the backdrop is the only close-on-click target', async () => {
    const onClose = vi.fn();
    render(<SettingsDrawer open onClose={onClose} deps={deps()} slots={{}} />);
    await screen.findByRole('dialog', { name: 'SETTINGS' });
    fireEvent.click(screen.getByTestId('sd-backdrop'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('reads its whole state from the core and never invents a default', async () => {
    const d = deps();
    render(<SettingsDrawer open onClose={vi.fn()} deps={d} slots={{}} />);
    // No switch from a group that reads `Settings` exists before the answer arrives.
    expect(screen.queryByRole('switch', { name: 'Start with the system' })).toBeNull();
    await waitFor(() => {
      expect(d.call).toHaveBeenCalledWith('settings.get', {});
    });
    expect(await screen.findByRole('switch', { name: 'Start with the system' })).toBeTruthy();
  });

  it('takes the position a write returns, not the one it optimistically set', async () => {
    const call = vi.fn((name: CommandName) => {
      // The core clamps: roasts stay on regardless of what was asked for.
      if (name === 'settings.set') return Promise.resolve({ ...settings, roastEnabled: true });
      return Promise.resolve(answers(name));
    });
    render(
      <SettingsDrawer open onClose={vi.fn()} deps={deps({ call: call as never })} slots={{}} />,
    );
    const roast = await screen.findByRole('switch', {
      name: 'Dry one-line notes on an opened project',
    });
    fireEvent.click(roast);
    await waitFor(() => {
      expect(call).toHaveBeenCalledWith('settings.set', {
        patch: toSettingsPatch({ roastEnabled: false }),
      });
    });
    expect(roast.getAttribute('aria-checked')).toBe('true');
  });

  it('sends only the field it was asked to change, and false is a value not an absence', () => {
    expect(toSettingsPatch({ roastEnabled: false })).toEqual({
      effectsTier: null,
      reducedMotionOverride: null,
      autostart: null,
      residentShortcut: null,
      roastEnabled: false,
      logLevel: null,
    });
    expect(toSettingsPatch({ effectsTier: 'off' }).effectsTier).toBe('off');
  });

  it('leaves a surface unknown when its read fails, rather than empty', async () => {
    const call = vi.fn((name: CommandName) => {
      if (name === 'roots.list') return Promise.reject(new Error('refused'));
      return Promise.resolve(answers(name));
    });
    render(
      <SettingsDrawer open onClose={vi.fn()} deps={deps({ call: call as never })} slots={{}} />,
    );
    await screen.findByRole('dialog', { name: 'SETTINGS' });
    await waitFor(() => {
      expect(call).toHaveBeenCalledWith('roots.list', {});
    });
    // §7.7a: a refused read is not an empty machine.
    expect(screen.getByText('—')).toBeTruthy();
    expect(screen.queryByText('0 OF 0 ACTIVE')).toBeNull();
  });

  it('animates nothing at the off tier', async () => {
    render(<SettingsDrawer open onClose={vi.fn()} deps={deps({ tier: 'off' })} slots={{}} />);
    const dialog = await screen.findByRole('dialog', { name: 'SETTINGS' });
    expect(dialog.style.animation).toBe('none');
    expect(screen.getByTestId('sd-backdrop').style.animation).toBe('none');
  });

  it('registers every group exactly once, in §11.3a order', () => {
    const groups = [...new Set(SETTINGS_ROWS.map((r) => r.group))];
    const positions = groups.map((g) => SETTINGS_GROUP_ORDER.indexOf(g));
    expect(positions.every((p) => p >= 0)).toBe(true);
    expect([...positions].sort((a, b) => a - b)).toEqual(positions);
    expect(new Set(SETTINGS_ROWS.map((r) => r.id)).size).toBe(SETTINGS_ROWS.length);
    expect(SETTINGS_ROWS.length).toBeGreaterThan(20);
  });
});

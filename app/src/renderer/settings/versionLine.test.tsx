import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import rootManifestRaw from '../../../../package.json?raw';
import type { CommandName, Settings } from '../../generated/protocol.js';
import type { CodothecaBridge } from '../../shared/bridge.js';
import { SettingsDrawer, type SettingsDrawerDeps } from './Drawer.js';

afterEach(cleanup);

const settings: Settings = {
  effectsTier: 'auto',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
  installRootId: null,
  contentScanEnabled: false,
  healthChecks: [],
};

const answers = (name: CommandName): unknown => {
  if (name === 'settings.get') return settings;
  if (name === 'targets.list') return { rows: [], resolved: null };
  if (name === 'identity.list' || name === 'roots.list') return [];
  return {};
};

/** The drawer's shell is the bridge itself at the mount point; this stub is that bridge's subset. */
const bridge = (
  buildStamp: string,
): Pick<
  CodothecaBridge,
  'pickRoot' | 'reveal' | 'indexLocation' | 'onShortcutState' | 'buildStamp'
> => ({
  pickRoot: vi.fn(() => Promise.resolve({ kind: 'cancelled' as const })),
  reveal: vi.fn(() => Promise.resolve({ ok: true, value: null })),
  indexLocation: vi.fn(() => Promise.resolve({ ok: true, value: null })),
  onShortcutState: vi.fn(),
  buildStamp,
});

const deps = (buildStamp: string): SettingsDrawerDeps => ({
  call: vi.fn((name: CommandName) => Promise.resolve(answers(name))) as never,
  shell: bridge(buildStamp),
  tier: 'full',
});

describe('the drawer footer', () => {
  it('AC-P4-48-25 the drawer renders the stamped version and artifact kind', async () => {
    expect(rootManifestRaw.length).toBeGreaterThan(0);
    const { version } = JSON.parse(rootManifestRaw) as { version: string };
    const line = `${version} · portable`;
    render(<SettingsDrawer open onClose={vi.fn()} deps={deps(line)} slots={{}} />);
    await screen.findByRole('dialog', { name: 'SETTINGS' });
    const drawn = screen.getByText(line);
    expect(drawn.closest('footer')).not.toBeNull();
  });

  it('draws no version line when the shell passed none', async () => {
    render(<SettingsDrawer open onClose={vi.fn()} deps={deps('')} slots={{}} />);
    const dialog = await screen.findByRole('dialog', { name: 'SETTINGS' });
    const footer = dialog.querySelector('footer');
    expect(footer).not.toBeNull();
    for (const span of footer?.querySelectorAll('span') ?? []) {
      expect(span.textContent).not.toBe('');
    }
  });
});

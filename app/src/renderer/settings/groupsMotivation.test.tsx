/**
 * §43.11's drawer group: one real switch for quiet mode, the two target rows §39 left without a
 * home, and nothing else. The targets are a game's settings, so quiet hides them; the switch
 * stays, or quiet could not be left.
 */
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { CommandName, Settings } from '../../generated/protocol.js';
import {
  FOCUS_TARGET_MAX_MINUTES,
  FOCUS_TARGET_MIN_MINUTES,
  RHYTHM_TARGET_MAX_DAYS,
  RHYTHM_TARGET_MIN_DAYS,
} from '../../shared/ringTargets.js';
import { QUIET_LIST } from '../../shared/motivationQuiet.js';
import { SettingsDrawer } from './Drawer.js';
import { MOTIVATION_GROUP_ROWS, MotivationGroups } from './groupsMotivation.js';
import { TARGET_UNSET } from './groupsScan.js';
import { SETTINGS_GROUP_ORDER } from './rows.js';

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
  focusTargetMinutes: 45,
  rhythmTarget: null,
  motivationShown: true,
};

const FOCUS_ROW = '[data-row="motivation-focus-target"]';
const RHYTHM_ROW = '[data-row="motivation-rhythm-target"]';

function group(): HTMLElement {
  const found = document.querySelector<HTMLElement>('[data-group="motivation"]');
  if (found === null) throw new Error('the motivation group is not drawn');
  return found;
}

/** The stepper buttons inside one row, by their visible glyph. */
function steps(row: string): { less: HTMLElement | null; more: HTMLElement | null } {
  const el = document.querySelector<HTMLElement>(row);
  if (el === null) throw new Error(`${row} is not drawn`);
  return {
    less: el.querySelector<HTMLElement>('[data-step="less"]'),
    more: el.querySelector<HTMLElement>('[data-step="more"]'),
  };
}

/** Click a stepper button the test expects to be drawn. */
function press(button: HTMLElement | null): void {
  if (button === null) throw new Error('the step is not drawn');
  fireEvent.click(button);
}

describe('the motivation group', () => {
  it('the switch patches motivationShown, checked meaning shown', () => {
    const onPatch = vi.fn();
    render(<MotivationGroups settings={settings} onPatch={onPatch} shown />);
    const control = within(group()).getByRole('switch');
    expect(control.getAttribute('aria-checked')).toBe('true');
    fireEvent.click(control);
    expect(onPatch).toHaveBeenCalledWith({ motivationShown: false });
  });

  it('the switch notes the closed list, read from its module', () => {
    render(<MotivationGroups settings={settings} onPatch={vi.fn()} shown />);
    const text = group().textContent;
    for (const entry of QUIET_LIST) expect(text).toContain(entry.label);
  });

  it('the targets patch their own fields by one step', () => {
    const onPatch = vi.fn();
    render(
      <MotivationGroups settings={{ ...settings, rhythmTarget: 3 }} onPatch={onPatch} shown />,
    );
    press(steps(FOCUS_ROW).more);
    press(steps(FOCUS_ROW).less);
    press(steps(RHYTHM_ROW).more);
    press(steps(RHYTHM_ROW).less);
    expect(onPatch.mock.calls).toEqual([
      [{ focusTargetMinutes: 60 }],
      [{ focusTargetMinutes: 30 }],
      [{ rhythmTarget: 4 }],
      [{ rhythmTarget: 2 }],
    ]);
  });

  it('never offers a step outside the mirrored range — absent, not disabled, at a bound', () => {
    const onPatch = vi.fn();
    const { rerender } = render(
      <MotivationGroups
        settings={{
          ...settings,
          focusTargetMinutes: FOCUS_TARGET_MIN_MINUTES,
          rhythmTarget: RHYTHM_TARGET_MIN_DAYS,
        }}
        onPatch={onPatch}
        shown
      />,
    );
    expect(steps(FOCUS_ROW).less).toBeNull();
    expect(steps(RHYTHM_ROW).less).toBeNull();
    rerender(
      <MotivationGroups
        settings={{
          ...settings,
          focusTargetMinutes: FOCUS_TARGET_MAX_MINUTES,
          rhythmTarget: RHYTHM_TARGET_MAX_DAYS,
        }}
        onPatch={onPatch}
        shown
      />,
    );
    expect(steps(FOCUS_ROW).more).toBeNull();
    expect(steps(RHYTHM_ROW).more).toBeNull();
    // A step from just inside a bound lands on the bound, never past it.
    rerender(
      <MotivationGroups
        settings={{ ...settings, focusTargetMinutes: FOCUS_TARGET_MAX_MINUTES - 5 }}
        onPatch={onPatch}
        shown
      />,
    );
    press(steps(FOCUS_ROW).more);
    expect(onPatch).toHaveBeenLastCalledWith({ focusTargetMinutes: FOCUS_TARGET_MAX_MINUTES });
  });

  it('renders an unset rhythm target as not set — never 0 — and steps it to the first day', () => {
    const onPatch = vi.fn();
    render(<MotivationGroups settings={settings} onPatch={onPatch} shown />);
    const row = document.querySelector<HTMLElement>(RHYTHM_ROW);
    expect(row?.textContent).not.toMatch(/\b0\b/u);
    expect(row?.querySelector('[data-value]')?.textContent).toBe(TARGET_UNSET);
    expect(steps(RHYTHM_ROW).less).toBeNull();
    press(steps(RHYTHM_ROW).more);
    expect(onPatch).toHaveBeenCalledWith({ rhythmTarget: RHYTHM_TARGET_MIN_DAYS });
  });

  it('under quiet the two target rows are absent and the switch is present', () => {
    render(
      <MotivationGroups
        settings={{ ...settings, motivationShown: false }}
        onPatch={vi.fn()}
        shown={false}
      />,
    );
    expect(document.querySelector(FOCUS_ROW)).toBeNull();
    expect(document.querySelector(RHYTHM_ROW)).toBeNull();
    const control = within(group()).getByRole('switch');
    expect(control.getAttribute('aria-checked')).toBe('false');
    expect(group().querySelectorAll('[data-motivation]')).toHaveLength(0);
  });

  it('marks each target row with the list entry that hides it', () => {
    render(<MotivationGroups settings={settings} onPatch={vi.fn()} shown />);
    const marked = [...group().querySelectorAll('[data-motivation]')].map((el) =>
      el.getAttribute('data-motivation'),
    );
    expect(marked).toEqual(['rings', 'rhythm']);
  });

  it('registers its rows under its own group, which sits directly after motion', () => {
    expect(MOTIVATION_GROUP_ROWS.length).toBe(3);
    expect(MOTIVATION_GROUP_ROWS.every((row) => row.group === 'motivation')).toBe(true);
    expect(SETTINGS_GROUP_ORDER.indexOf('motivation')).toBe(
      SETTINGS_GROUP_ORDER.indexOf('motion') + 1,
    );
  });

  it('is drawn in the drawer directly after the motion group', async () => {
    const call = vi.fn((name: CommandName) => {
      if (name === 'settings.get' || name === 'settings.set') return Promise.resolve(settings);
      if (name === 'targets.list') return Promise.resolve({ rows: [], resolved: null });
      if (name === 'identity.list' || name === 'roots.list') return Promise.resolve([]);
      return Promise.resolve({});
    });
    render(
      <SettingsDrawer
        open
        onClose={vi.fn()}
        deps={{
          call: call as never,
          shell: {
            pickRoot: vi.fn(() => Promise.resolve({ kind: 'cancelled' as const })),
            reveal: vi.fn(() => Promise.resolve({ ok: true, value: null })),
            indexLocation: vi.fn(() => Promise.resolve({ ok: true, value: null })),
            onShortcutState: vi.fn(),
            buildStamp: '',
          },
          tier: 'full',
        }}
        slots={{}}
      />,
    );
    await screen.findByRole('dialog', { name: 'SETTINGS' });
    await waitFor(() => {
      expect(document.querySelector('[data-group="motivation"]')).not.toBeNull();
    });
    const order = [...document.querySelectorAll('[data-group]')].map((el) =>
      el.getAttribute('data-group'),
    );
    expect(order[order.indexOf('motion') + 1]).toBe('motivation');
  });
});

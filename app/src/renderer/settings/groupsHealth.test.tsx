/**
 * [p3] §30.9 — one switch row per check, in the real drawer, over the list the core emits.
 *
 * The variant set is read from the **tracked** schema, never from `src/generated`, which is
 * gitignored. The core emits `healthChecks` in full, one entry per `DebtSource` variant, so the
 * fixture is built from that set — and a drawer that drew a list of its own would disagree with
 * it the day the enum moved.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
// `?raw` rather than node:fs: the renderer project carries no Node types by design.
import schemaRaw from '../../../../protocol/schema/protocol.json?raw';
import type {
  CommandName,
  DebtSource,
  HealthCheckSwitch,
  Settings,
} from '../../generated/protocol.js';
import { CONTENT_SCAN_LABEL } from '../../shared/contentScan.js';
import { SOURCE_LABELS } from '../project/health/labels.js';
import { SettingsDrawer, toSettingsPatch, type SettingsDrawerDeps } from './Drawer.js';
import { GRANT_MISSING_NOTE, HEALTH_CHECKS_FOOTNOTE } from './groupsHealth.js';

afterEach(cleanup);

const VARIANTS: readonly DebtSource[] = (() => {
  expect(schemaRaw.length).toBeGreaterThan(0);
  const raw = JSON.parse(schemaRaw) as { types?: Record<string, { variants?: string[] }> };
  return (raw.types?.['DebtSource']?.variants ?? []) as DebtSource[];
})();

const everyCheck = (enabled = true): HealthCheckSwitch[] =>
  VARIANTS.map((check) => ({ check, enabled }));

function settingsWith(over: Partial<Settings> = {}): Settings {
  return {
    effectsTier: 'auto',
    reducedMotionOverride: false,
    autostart: false,
    residentShortcut: null,
    roastEnabled: true,
    logLevel: 'info',
    installRootId: null,
    contentScanEnabled: true,
    healthChecks: everyCheck(),
    ...over,
  };
}

/** `set` answers what the core would store; by default the snapshot does not move. */
function deps(
  initial: Settings,
  set: (patch: unknown) => Settings = () => initial,
): { call: ReturnType<typeof vi.fn>; deps: SettingsDrawerDeps } {
  const call = vi.fn((name: CommandName, args: unknown) => {
    if (name === 'settings.get') return Promise.resolve(initial);
    if (name === 'settings.set') return Promise.resolve(set(args));
    if (name === 'targets.list') return Promise.resolve({ rows: [], resolved: null });
    if (name === 'identity.list' || name === 'roots.list') return Promise.resolve([]);
    return Promise.resolve({});
  });
  return {
    call,
    deps: {
      call: call as never,
      shell: {
        pickRoot: vi.fn(() => Promise.resolve({ kind: 'cancelled' as const })),
        reveal: vi.fn(() => Promise.resolve({ ok: true, value: null })),
        indexLocation: vi.fn(() =>
          Promise.resolve({ ok: true, value: { pathDisplay: '<data>/index.db', sizeBytes: 2048 } }),
        ),
        onShortcutState: vi.fn(),
      },
      tier: 'off',
    },
  };
}

async function openGroup(d: SettingsDrawerDeps): Promise<HTMLElement> {
  render(<SettingsDrawer open onClose={vi.fn()} deps={d} slots={{}} />);
  await screen.findByRole('dialog', { name: 'SETTINGS' });
  return waitFor(() => {
    const group = document.querySelector<HTMLElement>('[data-group="healthChecks"]');
    if (group === null) throw new Error('the health-check group has not mounted');
    return group;
  });
}

const rowsIn = (group: HTMLElement): HTMLElement[] => [
  ...group.querySelectorAll<HTMLElement>('[data-row^="health-check-"]'),
];

describe('§30.9 the per-check switches', () => {
  it('draws exactly one switch row per DebtSource variant', async () => {
    const group = await openGroup(deps(settingsWith()).deps);
    const rows = rowsIn(group);
    console.warn(
      `§30.9 DebtSource variants derived from the schema: ${String(VARIANTS.length)}; ` +
        `rows drawn: ${String(rows.length)}`,
    );
    expect(VARIANTS.length).toBeGreaterThan(0);
    expect(rows).toHaveLength(VARIANTS.length);
    for (const check of VARIANTS) {
      expect(group.querySelectorAll(`[data-row="health-check-${check}"]`), check).toHaveLength(1);
      // Named in words; the raw id stays in `data-row`, out of sight.
      const control = screen.getByRole('switch', { name: SOURCE_LABELS[check] });
      expect(control.getAttribute('aria-checked'), check).toBe('true');
      expect(
        group.querySelector(`[data-row="health-check-${check}"]`)?.textContent ?? '',
        check,
      ).not.toContain(check);
    }
  });

  it('draws the rows the core sends and holds no list of its own', async () => {
    const two: HealthCheckSwitch[] = [
      { check: 'missing_readme', enabled: true },
      { check: 'ci_red', enabled: false },
    ];
    const group = await openGroup(deps(settingsWith({ healthChecks: two })).deps);
    expect(rowsIn(group).map((row) => row.getAttribute('data-row'))).toEqual([
      'health-check-missing_readme',
      'health-check-ci_red',
    ]);
    expect(
      screen.getByRole('switch', { name: SOURCE_LABELS.ci_red }).getAttribute('aria-checked'),
    ).toBe('false');
  });

  it('switching a check off sends settings.set naming only that check, flipped', async () => {
    const { call, deps: d } = deps(settingsWith());
    await openGroup(d);
    fireEvent.click(screen.getByRole('switch', { name: SOURCE_LABELS.missing_tests }));
    await waitFor(() => {
      expect(call).toHaveBeenCalledWith('settings.set', {
        patch: toSettingsPatch({ healthChecks: [{ check: 'missing_tests', enabled: false }] }),
      });
    });
  });

  it('the caption says a check is off once the core has stored it, and the footnote says what off does', async () => {
    const initial = settingsWith();
    const stored = settingsWith({
      healthChecks: VARIANTS.map((check) => ({ check, enabled: check !== 'missing_tests' })),
    });
    const group = await openGroup(deps(initial, () => stored).deps);
    const caption = (): string =>
      group.querySelector('h2')?.parentElement?.lastElementChild?.textContent ?? '';
    const total = String(VARIANTS.length);
    expect(caption()).toBe(`${total} OF ${total} ON`);

    fireEvent.click(screen.getByRole('switch', { name: SOURCE_LABELS.missing_tests }));
    await waitFor(() => {
      expect(caption()).toBe(`${String(VARIANTS.length - 1)} OF ${total} ON`);
    });
    expect(
      screen
        .getByRole('switch', { name: SOURCE_LABELS.missing_tests })
        .getAttribute('aria-checked'),
    ).toBe('false');
    expect(HEALTH_CHECKS_FOOTNOTE).not.toBe('');
    expect(group.textContent ?? '').toContain(HEALTH_CHECKS_FOOTNOTE);
    expect(HEALTH_CHECKS_FOOTNOTE).toMatch(/hides its items/u);
    // §30.9: an item that survived the off period makes the check read open straight away; only
    // with no item left does it wait, unknown, for its next run. Half of that is not the rule.
    expect(HEALTH_CHECKS_FOOTNOTE).toMatch(/reads open if one of its items is still open/u);
    expect(HEALTH_CHECKS_FOOTNOTE).toMatch(/unknown until it next runs otherwise/u);
  });

  it('R142 the todo_marker row names the source-reading grant only while the switch is on and the grant is off', async () => {
    const noteOf = (group: HTMLElement): string =>
      group.querySelector('[data-row="health-check-todo_marker"]')?.textContent ?? '';
    expect(GRANT_MISSING_NOTE).not.toBe('');

    const ungranted = await openGroup(deps(settingsWith({ contentScanEnabled: false })).deps);
    expect(noteOf(ungranted)).toContain(GRANT_MISSING_NOTE);
    // The grant it points at is a real row in the same drawer, off.
    expect(
      screen.getByRole('switch', { name: CONTENT_SCAN_LABEL }).getAttribute('aria-checked'),
    ).toBe('false');
    // No other check needs that grant, so no other row carries the note.
    const carrying = rowsIn(ungranted).filter((row) =>
      (row.textContent ?? '').includes(GRANT_MISSING_NOTE),
    );
    expect(carrying.map((row) => row.getAttribute('data-row'))).toEqual([
      'health-check-todo_marker',
    ]);
    cleanup();

    const granted = await openGroup(deps(settingsWith({ contentScanEnabled: true })).deps);
    expect(noteOf(granted)).not.toContain(GRANT_MISSING_NOTE);
    cleanup();

    // The switch wins when both apply: a check the user turned off is not waiting on a grant.
    const switchedOff = await openGroup(
      deps(
        settingsWith({
          contentScanEnabled: false,
          healthChecks: VARIANTS.map((check) => ({ check, enabled: check !== 'todo_marker' })),
        }),
      ).deps,
    );
    expect(noteOf(switchedOff)).not.toContain(GRANT_MISSING_NOTE);
  });

  it('the words clean healthy none and all appear nowhere in the group', async () => {
    const group = await openGroup(deps(settingsWith({ contentScanEnabled: false })).deps);
    const text = group.textContent ?? '';
    expect(text).not.toBe('');
    const names = [...group.querySelectorAll('*')]
      .map((n) => n.getAttribute('aria-label') ?? '')
      .join(' ');
    for (const banned of ['clean', 'healthy', 'none', 'all']) {
      const pattern = new RegExp(`\\b${banned}\\b`, 'iu');
      expect(text).not.toMatch(pattern);
      expect(names).not.toMatch(pattern);
    }
  });
});

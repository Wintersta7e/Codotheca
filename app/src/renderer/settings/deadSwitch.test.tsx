import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
// `?raw` rather than node:fs: the renderer project carries no Node types by design, and under
// jsdom `import.meta.url` is not a file URL.
import schemaRaw from '../../../../protocol/schema/protocol.json?raw';
import type { CommandName, Settings } from '../../generated/protocol.js';
import { IPC_PICK_ROOT, IPC_REVEAL } from '../../shared/channels.js';
import { SETTINGS_ROWS, SettingsDrawer, type SettingsDrawerDeps } from './Drawer.js';
import { isControl, type SettingsSlots } from './rows.js';

afterEach(cleanup);

/**
 * The command vocabulary is read from the **tracked** schema, not from `src/generated`, which is
 * gitignored — a check that greps an ignored path is the defect this project has already shipped
 * once. This is also the cross-language mirror test R24 asks for: the drawer's rows are checked
 * against the file both languages generate from.
 */
const SCHEMA_COMMANDS: readonly string[] = (() => {
  expect(schemaRaw.length).toBeGreaterThan(0);
  const raw: unknown = JSON.parse(schemaRaw);
  const commands = (raw as { commands?: { name: string }[] }).commands ?? [];
  if (commands.length === 0) throw new Error('protocol.json declares no command');
  return commands.map((command) => command.name);
})();

const SHELL_CHANNELS = new Set<string>([IPC_PICK_ROOT, IPC_REVEAL]);

const settings: Settings = {
  effectsTier: 'auto',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
  installRootId: null,
  contentScanEnabled: false,
  // [p3] Two of §30.9's rows, one each way, so the per-check switches are in the audit below.
  healthChecks: [
    { check: 'todo_marker', enabled: true },
    { check: 'missing_readme', enabled: false },
  ],
};

/** Every slot supplied, so every row the registry declares is actually drawn. */
const slots = (): Required<SettingsSlots> => ({
  chooseProjectToHide: vi.fn(),
  chooseLaunchTarget: vi.fn(),
  identityCard: () => null,
  addIdentityAddress: vi.fn(),
  githubPanel: () => null,
});

function harness(): {
  call: ReturnType<typeof vi.fn>;
  reveal: ReturnType<typeof vi.fn>;
  deps: SettingsDrawerDeps;
} {
  const call = vi.fn((name: CommandName) => {
    if (name === 'settings.get' || name === 'settings.set') return Promise.resolve(settings);
    if (name === 'targets.list') return Promise.resolve({ rows: [], resolved: null });
    if (name === 'identity.list') return Promise.resolve([]);
    if (name === 'roots.list') return Promise.resolve([]);
    return Promise.resolve({});
  });
  const reveal = vi.fn(() => Promise.resolve({ ok: true, value: null }));
  return {
    call,
    reveal,
    deps: {
      call: call as never,
      shell: {
        pickRoot: vi.fn(() => Promise.resolve({ kind: 'cancelled' as const })),
        reveal,
        indexLocation: vi.fn(() =>
          Promise.resolve({ ok: true, value: { pathDisplay: '<data>/index.db', sizeBytes: 2048 } }),
        ),
        onShortcutState: vi.fn(),
      },
      tier: 'full',
    },
  };
}

async function open(
  deps: SettingsDrawerDeps,
  given: SettingsSlots = slots(),
): Promise<HTMLElement> {
  const { container } = render(<SettingsDrawer open onClose={vi.fn()} deps={deps} slots={given} />);
  await screen.findByRole('dialog', { name: 'SETTINGS' });
  // The groups that read `Settings` mount on its answer, so the audit must wait for it.
  await waitFor(() => {
    expect(screen.getAllByRole('switch').length).toBeGreaterThan(3);
  });
  return container;
}

describe('the dead-switch rule, over the whole drawer', () => {
  it('every control names a command, a channel or a slot that exists', () => {
    const supplied = new Set(Object.keys(slots()));
    let checked = 0;
    for (const spec of SETTINGS_ROWS) {
      switch (spec.backing.kind) {
        case 'command':
          expect(SCHEMA_COMMANDS, spec.id).toContain(spec.backing.command);
          checked += 1;
          break;
        case 'shell':
          expect(SHELL_CHANNELS.has(spec.backing.channel), spec.id).toBe(true);
          checked += 1;
          break;
        case 'host':
          // Checked against what the host can actually pass, not against a restated list.
          expect(supplied.has(spec.backing.slot), spec.id).toBe(true);
          checked += 1;
          break;
        case 'statement':
          break;
      }
    }
    expect(checked).toBeGreaterThan(10);
  });

  it('draws every row it registers, once the host supplies every slot', async () => {
    const { deps } = harness();
    const container = await open(deps);
    // Templates are cloned per root and per check, under their own ids. There is no root here;
    // the check rows are the health group's own test (`groupsHealth.test.tsx`).
    const templates = new Set(['roots-enabled', 'roots-descend', 'health-check']);
    for (const spec of SETTINGS_ROWS) {
      if (templates.has(spec.id)) continue;
      expect(container.querySelector(`[data-row="${spec.id}"]`), spec.id).not.toBeNull();
    }
  });

  it('every rendered switch is named and does something observable when operated', async () => {
    const { call, reveal, deps } = harness();
    await open(deps);
    const switches = screen.getAllByRole('switch');
    expect(switches.length).toBeGreaterThan(3);
    let issuedCall = 0;
    let changedLocally = 0;
    for (const control of switches) {
      const name = control.getAttribute('aria-label') ?? '';
      expect(name).not.toBe('');
      const before = call.mock.calls.length + reveal.mock.calls.length;
      const checkedBefore = control.getAttribute('aria-checked');
      fireEvent.click(control);
      const after = call.mock.calls.length + reveal.mock.calls.length;
      const moved = after > before;
      const flipped = control.getAttribute('aria-checked') !== checkedBefore;
      // §11.3a: "Every switch must do something observable." A switch whose click produces
      // neither a call nor a change in its own state is exactly the defect this rule names.
      expect(moved || flipped, name).toBe(true);
      if (moved) issuedCall += 1;
      if (flipped) changedLocally += 1;
    }
    // Both halves of the disjunction are exercised, so neither is carrying the other.
    expect(issuedCall).toBeGreaterThan(0);
    expect(changedLocally).toBeGreaterThan(0);
  });

  it('no statement row announces as a control anywhere in the drawer', async () => {
    const { deps } = harness();
    const container = await open(deps);
    let audited = 0;
    for (const spec of SETTINGS_ROWS.filter((row) => !isControl(row.backing))) {
      const row = container.querySelector(`[data-row="${spec.id}"]`);
      if (row === null) continue; // a group without its slot is not drawn at all — also legal
      audited += 1;
      expect(row.querySelector('[role="switch"], button, input, [tabindex]'), spec.id).toBeNull();
      expect(row.querySelector('[aria-disabled]'), spec.id).toBeNull();
      expect(row.querySelector('[aria-checked]'), spec.id).toBeNull();
    }
    expect(audited).toBeGreaterThan(6);
  });

  it('phase 1 has no destructive control, in any spelling, on this surface', async () => {
    const { deps } = harness();
    await open(deps);
    const text = document.body.textContent;
    // The token appears in no rendered string at all.
    for (const banned of [/\bforget\b/i, /\buninstall\b/i]) expect(text).not.toMatch(banned);
    // And no control offers to destroy anything. `NEVER delete_repo` is a scope this product
    // promises never to request; it is a span in the GitHub statement, never a control.
    const controls = [...screen.getAllByRole('button'), ...screen.getAllByRole('switch')];
    expect(controls.length).toBeGreaterThan(4);
    for (const control of controls) {
      const name = control.getAttribute('aria-label') ?? control.textContent;
      for (const banned of [/\bforget\b/i, /\buninstall\b/i, /\bdelete\b/i, /\bremove\b/i]) {
        expect(name).not.toMatch(banned);
      }
    }
    // §17: no command this drawer can issue is destructive.
    const issued = new Set(
      SETTINGS_ROWS.flatMap((row) => (row.backing.kind === 'command' ? [row.backing.command] : [])),
    );
    for (const forbidden of ['roots.remove', 'locations.relocate', 'projects.merge']) {
      expect(issued.has(forbidden as CommandName), forbidden).toBe(false);
    }
  });

  it('no field on this surface accepts a filesystem path or an executable', async () => {
    const { deps } = harness();
    const container = await open(deps);
    // §2.4: paths and executables enter only from a shell-owned native dialog.
    expect(screen.queryAllByRole('textbox')).toHaveLength(0);
    expect(container.querySelectorAll('input, textarea')).toHaveLength(0);
  });
});

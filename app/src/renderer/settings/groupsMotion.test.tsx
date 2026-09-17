import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Settings } from '../../generated/protocol.js';
import { MOTION_GROUP_ROWS, MotionGroups, effectiveTierNote } from './groupsMotion.js';

afterEach(cleanup);

const settings: Settings = {
  effectsTier: 'auto',
  reducedMotionOverride: false,
  autostart: false,
  residentShortcut: null,
  roastEnabled: true,
  logLevel: 'info',
  installRootId: null,
};

const props = {
  settings,
  shortcut: { chord: null, registered: false },
  recording: false,
  onPatch: vi.fn(),
  onRecordChord: vi.fn(),
  onChordCaptured: vi.fn(),
};

describe('group 5, MOTION', () => {
  it('offers exactly four tiers and no fifth control', () => {
    const onPatch = vi.fn();
    render(<MotionGroups {...props} onPatch={onPatch} />);
    const group = screen.getByRole('group', { name: 'MOTION' });
    for (const tier of ['AUTO', 'FULL', 'REDUCED', 'OFF']) {
      expect(screen.getByRole('radio', { name: tier })).toBeTruthy();
    }
    expect(screen.getAllByRole('radio')).toHaveLength(4);
    fireEvent.click(screen.getByRole('radio', { name: 'REDUCED' }));
    expect(onPatch).toHaveBeenCalledWith({ effectsTier: 'reduced' });
    // §11.6 rides the tier; a per-effect boolean beside a per-effect tier is two controls for
    // one behaviour, which is exactly how dead switches shipped twice before.
    for (const cut of [/flicker/i, /ambient/i, /decay/i, /drift/i, /level/i, /quest/i]) {
      expect(group.textContent).not.toMatch(cut);
    }
  });

  it('reports which tier is current, so the group is not a set of four unlit buttons', () => {
    render(<MotionGroups {...props} settings={{ ...settings, effectsTier: 'full' }} />);
    expect(screen.getByRole('radio', { name: 'FULL' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'AUTO' }).getAttribute('aria-checked')).toBe('false');
  });

  it('says the override clamps the tier rather than silently overriding the choice', () => {
    const clamped: Settings = { ...settings, effectsTier: 'full', reducedMotionOverride: true };
    expect(effectiveTierNote(clamped)).toContain('REDUCED');
    expect(effectiveTierNote(settings)).toBeNull();
    // Already at or below reduced, so there is nothing to say.
    expect(effectiveTierNote({ ...clamped, effectsTier: 'off' })).toBeNull();
    render(<MotionGroups {...props} settings={clamped} />);
    expect(screen.getByText(/CLAMPED TO REDUCED/)).toBeTruthy();
  });

  it('states density and names its home without drawing a second control for it', () => {
    render(<MotionGroups {...props} />);
    const density = MOTION_GROUP_ROWS.find((r) => r.id === 'motion-density');
    expect(density?.backing).toEqual({ kind: 'statement' });
    expect(screen.getByText(/TOP BAR/)).toBeTruthy();
    expect(screen.getByRole('group', { name: 'MOTION' }).textContent).toContain('LARGE');
  });
});

describe('group 6, RESIDENCY', () => {
  it('the hybrid is a statement and only two rows in the group are controls', () => {
    render(<MotionGroups {...props} />);
    const residency = MOTION_GROUP_ROWS.filter((r) => r.group === 'residency');
    const controls = residency.filter((r) => r.backing.kind !== 'statement');
    expect(controls.map((r) => r.id)).toEqual(['residency-autostart', 'residency-chord']);
    expect(
      screen.getByRole('switch', { name: 'Start with the system' }).getAttribute('aria-checked'),
    ).toBe('false');
  });

  it('carries the measured numbers, not the 30 MB the consent copy once claimed', () => {
    render(<MotionGroups {...props} />);
    expect(
      screen.getByText(
        '307 MB EMPTY · 522 MB WITH A FULL SHELF · 232 MB WITH THE WINDOW DESTROYED',
      ),
    ).toBeTruthy();
    expect(screen.queryByText(/30 MB/)).toBeNull();
  });

  it('ships the chord unbound and never pre-fills one', () => {
    render(<MotionGroups {...props} />);
    expect(screen.getByText('NOT SET')).toBeTruthy();
  });

  it('a refused chord reads back the reason and never renders as bound', () => {
    render(<MotionGroups {...props} shortcut={{ chord: 'Control+Shift+K', registered: false }} />);
    expect(
      screen.getByText('NOT SET · Control+Shift+K IS HELD BY ANOTHER APPLICATION'),
    ).toBeTruthy();
  });

  it('a captured chord is reported once, and Escape cancels without binding', () => {
    const onChordCaptured = vi.fn();
    render(<MotionGroups {...props} recording onChordCaptured={onChordCaptured} />);
    const recorder = screen.getByRole('button', { name: /PRESS A CHORD/ });
    fireEvent.keyDown(recorder, { key: 'k', code: 'KeyK', ctrlKey: true, shiftKey: true });
    expect(onChordCaptured).toHaveBeenCalledWith('Control+Shift+K');
    fireEvent.keyDown(recorder, { key: 'Escape', code: 'Escape' });
    expect(onChordCaptured).toHaveBeenLastCalledWith(null);
    expect(onChordCaptured).toHaveBeenCalledTimes(2);
  });

  it('holding only a modifier binds nothing and keeps the recorder listening', () => {
    const onChordCaptured = vi.fn();
    render(<MotionGroups {...props} recording onChordCaptured={onChordCaptured} />);
    const recorder = screen.getByRole('button', { name: /PRESS A CHORD/ });
    fireEvent.keyDown(recorder, { key: 'Control', code: 'ControlLeft', ctrlKey: true });
    expect(onChordCaptured).not.toHaveBeenCalled();
  });

  it('does not let the recorder key reach the drawer, which would close it mid-recording', () => {
    const onChordCaptured = vi.fn();
    const outer = vi.fn();
    render(
      <div onKeyDown={outer}>
        <MotionGroups {...props} recording onChordCaptured={onChordCaptured} />
      </div>,
    );
    fireEvent.keyDown(screen.getByRole('button', { name: /PRESS A CHORD/ }), {
      key: 'Escape',
      code: 'Escape',
    });
    expect(onChordCaptured).toHaveBeenCalledWith(null);
    expect(outer).not.toHaveBeenCalled();
  });

  it('states the consequence of the system window menu chord rather than swallowing it', () => {
    const onChordCaptured = vi.fn();
    render(<MotionGroups {...props} recording onChordCaptured={onChordCaptured} />);
    fireEvent.keyDown(screen.getByRole('button', { name: /PRESS A CHORD/ }), {
      key: ' ',
      code: 'Space',
      altKey: true,
    });
    expect(onChordCaptured).toHaveBeenCalledWith('Alt+Space');
    expect(screen.getByText(/LOSES ITS ALT\+SPACE MENU/)).toBeTruthy();
  });
});

describe('group 7, PROJECT PAGE', () => {
  it('is one real switch, defaulting on, with the invariant as its note', () => {
    const onPatch = vi.fn();
    render(<MotionGroups {...props} onPatch={onPatch} />);
    const roast = screen.getByRole('switch', { name: 'Dry one-line notes on an opened project' });
    expect(roast.getAttribute('aria-checked')).toBe('true');
    expect(screen.getByText('NEVER ON THE SHELF · NEVER DURING TRIAGE')).toBeTruthy();
    fireEvent.click(roast);
    expect(onPatch).toHaveBeenCalledWith({ roastEnabled: false });
  });
});

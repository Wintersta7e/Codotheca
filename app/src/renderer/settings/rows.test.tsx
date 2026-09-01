import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  SETTINGS_GROUP_ORDER,
  SettingsGroup,
  SettingsRow,
  SwitchRow,
  isControl,
  type SettingsRowSpec,
} from './rows.js';

afterEach(cleanup);

const controlSpec: SettingsRowSpec = {
  id: 'projectpage-roast',
  group: 'projectPage',
  label: 'Dry one-line notes on an opened project',
  note: 'NEVER ON THE SHELF · NEVER DURING TRIAGE',
  backing: { kind: 'command', command: 'settings.set' },
};

const statementSpec: SettingsRowSpec = {
  id: 'residency-hybrid',
  group: 'residency',
  label: 'The window is destroyed 30 minutes after last use',
  note: '9 MS TO SHOW WHILE THE WINDOW LIVES · 134 MS AFTER IT IS DESTROYED',
  backing: { kind: 'statement' },
};

describe('the two row variants, and no third', () => {
  it('a control row is a switch that reports its state and fires its handler', () => {
    const onChange = vi.fn();
    render(<SwitchRow spec={controlSpec} checked={false} onChange={onChange} />);
    const control = screen.getByRole('switch', { name: controlSpec.label });
    expect(control.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(control);
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it('a statement row exposes no control at all — not a disabled one', () => {
    const { container } = render(<SwitchRow spec={statementSpec} on />);
    expect(screen.queryByRole('switch')).toBeNull();
    expect(screen.queryByRole('checkbox')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
    // §11.7: an accessibility tree that names a control which cannot act is the same defect one
    // layer down. Not focusable, and never aria-disabled — that would still announce.
    expect(container.querySelector('[tabindex]')).toBeNull();
    expect(container.querySelector('[aria-disabled]')).toBeNull();
    expect(container.querySelector('[aria-checked]')).toBeNull();
    expect(container.querySelector('button')).toBeNull();
  });

  it('a statement still reads its text, so the setting is discoverable', () => {
    render(<SwitchRow spec={statementSpec} on />);
    expect(screen.getByText(statementSpec.label)).toBeTruthy();
    expect(screen.getByText(statementSpec.note as string)).toBeTruthy();
  });

  it('isControl agrees with what the row renders, for every backing kind', () => {
    expect(isControl({ kind: 'command', command: 'settings.set' })).toBe(true);
    expect(isControl({ kind: 'shell', channel: 'codotheca:pick-root' })).toBe(true);
    expect(isControl({ kind: 'host', slot: 'chooseProjectToHide' })).toBe(true);
    expect(isControl({ kind: 'statement' })).toBe(false);
  });

  it('names the row in the DOM so an audit can find it by its spec id', () => {
    const { container } = render(<SwitchRow spec={statementSpec} on />);
    expect(container.querySelector(`[data-row="${statementSpec.id}"]`)).toBeTruthy();
  });

  it('renders a value and a trailing control when the row carries them', () => {
    render(
      <SettingsRow
        spec={{ ...controlSpec, id: 'data-index' }}
        value="<data>/index.db"
        control={<button type="button">REVEAL</button>}
      />,
    );
    expect(screen.getByText('<data>/index.db')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'REVEAL' })).toBeTruthy();
  });
});

describe('a settings group', () => {
  it('titles itself, qualifies itself once, and names nothing else', () => {
    render(
      <SettingsGroup id="excluded" title="EXCLUDED FROM EVERY SCAN" caption="A CAPTION">
        <SettingsRow spec={statementSpec} />
      </SettingsGroup>,
    );
    const group = screen.getByRole('group', { name: 'EXCLUDED FROM EVERY SCAN' });
    expect(within(group).getByText('A CAPTION')).toBeTruthy();
    expect(within(group).getAllByText('A CAPTION')).toHaveLength(1);
  });

  it('orders every group §11.3a declares, once each', () => {
    expect(new Set(SETTINGS_GROUP_ORDER).size).toBe(SETTINGS_GROUP_ORDER.length);
    expect(SETTINGS_GROUP_ORDER[0]).toBe('roots');
    expect(SETTINGS_GROUP_ORDER.indexOf('identity')).toBeGreaterThan(
      SETTINGS_GROUP_ORDER.indexOf('data'),
    );
    expect(SETTINGS_GROUP_ORDER.indexOf('notifications')).toBe(SETTINGS_GROUP_ORDER.length - 1);
  });
});

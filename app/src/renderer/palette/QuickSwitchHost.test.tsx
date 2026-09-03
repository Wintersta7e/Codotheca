import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { LocationId, ProjectId, ProjectRow } from '../../generated/protocol.js';
import { makeLocationRef, makeProjectRow } from '../testing/projectRow.js';
import { QuickSwitchHost } from './QuickSwitchHost.js';
import type { QuickSwitchHostProps } from './QuickSwitchHost.js';

afterEach(cleanup);

const rows: ProjectRow[] = [
  makeProjectRow({
    id: 1 as ProjectId,
    name: 'Alpha',
    lastTouchedAt: 900,
    primaryLocation: makeLocationRef(11),
  }),
  makeProjectRow({
    id: 2 as ProjectId,
    name: 'Beta',
    lastTouchedAt: 800,
    primaryLocation: makeLocationRef(12),
  }),
];

function host(over: Partial<QuickSwitchHostProps> = {}): {
  props: QuickSwitchHostProps;
} & ReturnType<typeof render> {
  const props: QuickSwitchHostProps = {
    rows,
    liveSessionProjectIds: new Set<ProjectId>(),
    effectsTier: 'off',
    jewelFor: () => 'oklch(0.6 0.17 26)',
    now: () => 1_800_000_000,
    onLaunch: vi.fn(),
    onOpenPage: vi.fn(),
    ...over,
  };
  return { props, ...render(<QuickSwitchHost {...props} />) };
}

const altSpace = (target: Element | Window = window): void => {
  fireEvent.keyDown(target, { code: 'Space', altKey: true });
};

describe('QuickSwitchHost', () => {
  it('renders nothing until the chord is pressed', () => {
    host();
    expect(document.querySelector('.qs-panel')).toBeNull();
    altSpace();
    expect(document.querySelector('.qs-panel')).not.toBeNull();
  });

  // Criterion 32: the keystroke never reaches a focused field.
  it('takes the chord while a field has focus, and prevents the default', () => {
    host();
    const field = document.createElement('input');
    document.body.append(field);
    field.focus();
    const event = new KeyboardEvent('keydown', {
      code: 'Space',
      altKey: true,
      bubbles: true,
      cancelable: true,
    });
    // A raw dispatch rather than `fireEvent`, because the assertion is on the event object
    // itself; `act` is what `fireEvent` would otherwise have supplied.
    act(() => {
      field.dispatchEvent(event);
    });
    expect(event.defaultPrevented).toBe(true);
    expect(document.querySelector('.qs-panel')).not.toBeNull();
    field.remove();
  });

  it('leaves an unclaimed key alone', () => {
    host();
    const event = new KeyboardEvent('keydown', { code: 'KeyJ', bubbles: true, cancelable: true });
    window.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
  });

  // R42: while closed the host resolves in the *ambient* context, where ArrowDown is the shelf's
  // key and the palette maps none of it. It must neither act on it nor swallow it.
  it('leaves the ambient context’s own keys to the ambient context', () => {
    host();
    const event = new KeyboardEvent('keydown', {
      code: 'ArrowDown',
      bubbles: true,
      cancelable: true,
    });
    window.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
    expect(document.querySelector('.qs-panel')).toBeNull();
  });

  it('moves the cursor, launches with Enter and closes', () => {
    const { props } = host();
    altSpace();
    fireEvent.keyDown(window, { code: 'ArrowDown' });
    fireEvent.keyDown(window, { code: 'Enter' });
    expect(props.onLaunch).toHaveBeenCalledWith(2 as ProjectId, 12 as LocationId);
    expect(document.querySelector('.qs-panel')).toBeNull();
  });

  // R45: the palette's query bar is autofocused, so in the product every one of its keys arrives
  // with a text-entry target. A suite that only ever dispatches on `window` sees `target: null`
  // and would stay green over a palette that cannot be navigated at all.
  it('works from the query field, which is where the keys actually land', () => {
    const { props } = host();
    altSpace();
    const field = screen.getByRole('combobox');
    expect(document.activeElement).toBe(field);
    fireEvent.keyDown(field, { code: 'ArrowDown' });
    fireEvent.keyDown(field, { code: 'Enter' });
    expect(props.onLaunch).toHaveBeenCalledWith(2 as ProjectId, 12 as LocationId);
    expect(document.querySelector('.qs-panel')).toBeNull();
  });

  it('Shift+Enter opens the page instead of launching', () => {
    const { props } = host();
    altSpace();
    fireEvent.keyDown(window, { code: 'Enter', shiftKey: true });
    expect(props.onOpenPage).toHaveBeenCalledWith(1 as ProjectId);
    expect(props.onLaunch).not.toHaveBeenCalled();
    expect(document.querySelector('.qs-panel')).toBeNull();
  });

  it('Enter opens the page when the selected row has no local copy', () => {
    const unavailable = makeProjectRow({
      id: 3 as ProjectId,
      name: 'Remote only',
      primaryLocation: null,
    });
    const { props } = host({ rows: [unavailable] });
    altSpace();
    fireEvent.keyDown(screen.getByRole('combobox'), { code: 'Enter' });
    expect(props.onOpenPage).toHaveBeenCalledWith(3 as ProjectId);
    expect(props.onLaunch).not.toHaveBeenCalled();
    expect(document.querySelector('.qs-panel')).toBeNull();
  });

  it('Esc closes and restores focus to whatever held it', () => {
    host();
    const field = document.createElement('input');
    document.body.append(field);
    field.focus();
    altSpace(field);
    expect(document.activeElement).not.toBe(field);
    fireEvent.keyDown(window, { code: 'Escape' });
    expect(document.querySelector('.qs-panel')).toBeNull();
    expect(document.activeElement).toBe(field);
    field.remove();
  });

  it('opens empty every time, carrying nothing over', () => {
    host();
    altSpace();
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'bet' } });
    expect(screen.getAllByRole('option')).toHaveLength(1);
    fireEvent.keyDown(window, { code: 'Escape' });
    altSpace();
    expect(screen.getByRole<HTMLInputElement>('combobox').value).toBe('');
    expect(screen.getAllByRole('option')).toHaveLength(2);
  });

  it('the shell channel opens it, and a second delivery does not re-mount the panel', () => {
    let fire: (() => void) | null = null;
    host({
      subscribeShellOpen: (cb) => {
        fire = cb;
        return () => {
          fire = null;
        };
      },
    });
    const open = fire as (() => void) | null;
    if (open === null) throw new Error('the host never subscribed to the shell channel');
    act(() => {
      open();
    });
    const first = document.querySelector('.qs-panel');
    expect(first).not.toBeNull();
    act(() => {
      open();
    });
    expect(document.querySelector('.qs-panel')).toBe(first);
  });
});

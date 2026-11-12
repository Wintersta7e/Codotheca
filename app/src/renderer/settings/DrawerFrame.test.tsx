import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DrawerFrame } from './DrawerFrame.js';
import { DRAWER_WIDTH_PX } from './styles.js';

afterEach(() => {
  cleanup();
});

function mount(onClose = vi.fn(), onKeyDown = vi.fn()): HTMLElement {
  render(
    <DrawerFrame
      tier="off"
      label="Probe drawer"
      backdropTestId="probe-backdrop"
      onClose={onClose}
      onKeyDown={onKeyDown}
    >
      <button type="button">INSIDE</button>
    </DrawerFrame>,
  );
  return screen.getByRole('dialog', { name: 'Probe drawer' });
}

describe('§11.3a’s drawer chrome, one owner for both drawers', () => {
  it('holds a 400px panel at the right edge of a fixed scrim, as resolved styles', () => {
    const panel = mount();
    const backdrop = screen.getByTestId('probe-backdrop');
    const scrim = getComputedStyle(backdrop);
    expect(scrim.position).toBe('fixed');
    expect(scrim.justifyContent).toBe('flex-end');
    expect(scrim.inset === '0px' || scrim.top === '0px').toBe(true);
    expect(getComputedStyle(panel).width).toBe(`${String(DRAWER_WIDTH_PX)}px`);
    expect(panel.parentElement).toBe(backdrop);
  });

  it('closes on the backdrop and never on a click inside the panel', () => {
    const onClose = vi.fn();
    mount(onClose);
    fireEvent.click(screen.getByRole('button', { name: 'INSIDE' }));
    expect(onClose).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('probe-backdrop'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('keeps a key inside the panel from reaching the shelf’s window listener', () => {
    const onKeyDown = vi.fn();
    mount(vi.fn(), onKeyDown);
    // The shelf's grid listens on the window in the bubble phase; the palette, in the capture.
    const bubbling = vi.fn();
    const capturing = vi.fn();
    window.addEventListener('keydown', bubbling);
    window.addEventListener('keydown', capturing, true);
    try {
      fireEvent.keyDown(screen.getByRole('button', { name: 'INSIDE' }), {
        key: 'Enter',
        code: 'Enter',
      });
    } finally {
      window.removeEventListener('keydown', bubbling);
      window.removeEventListener('keydown', capturing, true);
    }
    expect(onKeyDown).toHaveBeenCalledTimes(1);
    expect(capturing).toHaveBeenCalledTimes(1);
    expect(bubbling).not.toHaveBeenCalled();
  });
});

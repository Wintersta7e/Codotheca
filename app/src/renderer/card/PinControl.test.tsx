import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import * as pinModule from './PinControl';
import { GRID_TILE_BANDS, HERO_BANDS } from './geometry';
import { PIN_ROTATION, PinControl } from './PinControl';

afterEach(cleanup);

interface DrawOptions {
  readonly isPinned?: boolean;
  readonly surface?: 'card' | 'hero';
  readonly visible?: boolean;
  readonly projectName?: string;
}

const draw = (over: DrawOptions = {}): (() => void) => {
  const onToggle = vi.fn();
  render(
    <PinControl
      projectName={over.projectName ?? 'Aurora'}
      isPinned={over.isPinned ?? false}
      surface={over.surface ?? 'card'}
      visible={over.visible ?? false}
      onToggle={onToggle}
    />,
  );
  return onToggle;
};

describe('it is a real button carrying its state', () => {
  it('exposes aria-pressed rather than a second hidden string', () => {
    draw({ isPinned: true });
    const button = screen.getByRole('button', { name: 'Unpin Aurora' });
    expect(button.getAttribute('aria-pressed')).toBe('true');
    expect(button.tagName).toBe('BUTTON');
    expect(button.getAttribute('type')).toBe('button');
  });

  it('names the act and the subject in both directions', () => {
    draw({ isPinned: false });
    expect(screen.getByRole('button', { name: 'Pin Aurora' })).toBeTruthy();
    expect(screen.getByRole('button').getAttribute('aria-pressed')).toBe('false');
  });

  it('is reached by P and not by Tab, so the roving tabindex stays on the card', () => {
    draw();
    expect(screen.getByRole('button').getAttribute('tabindex')).toBe('-1');
  });
});

describe('the hit target is the one §7.8a states for that surface', () => {
  it('takes the tile numbers on the tile', () => {
    draw();
    const button = screen.getByRole('button');
    expect(button.style.width).toBe(`${String(GRID_TILE_BANDS.pin.hit)}px`);
    expect(button.style.right).toBe(`${String(GRID_TILE_BANDS.pin.hitRight)}px`);
    expect(button.style.top).toBe(`${String(GRID_TILE_BANDS.pin.hitTop)}px`);
  });

  it('takes the hero numbers on the hero — one geometry across both fails a correct hero', () => {
    draw({ surface: 'hero' });
    const button = screen.getByRole('button');
    expect(button.style.width).toBe(`${String(HERO_BANDS.pin.hit)}px`);
    expect(button.style.right).toBe(`${String(HERO_BANDS.pin.hitRight)}px`);
    expect(HERO_BANDS.pin.hit).not.toBe(GRID_TILE_BANDS.pin.hit);
  });
});

describe('the silhouette is two rectangles', () => {
  it('draws a bar and a shaft at the stated sizes, rotated about the box centre', () => {
    draw({ isPinned: true });
    const bar = screen.getByTestId('cdt-pin-bar');
    const shaft = screen.getByTestId('cdt-pin-shaft');
    expect(bar.style.width).toBe(`${String(GRID_TILE_BANDS.pin.barW)}px`);
    expect(bar.style.height).toBe(`${String(GRID_TILE_BANDS.pin.barH)}px`);
    expect(shaft.style.width).toBe(`${String(GRID_TILE_BANDS.pin.shaftW)}px`);
    expect(shaft.style.height).toBe(`${String(GRID_TILE_BANDS.pin.shaftH)}px`);
    expect(screen.getByTestId('cdt-pin-silhouette').style.transform).toBe(PIN_ROTATION);
  });

  it('uses no icon font and no image, because the bundle guards three families', () => {
    const { container } = render(
      <PinControl projectName="A" isPinned surface="card" visible onToggle={() => {}} />,
    );
    expect(container.querySelector('svg')).toBeNull();
    expect(container.querySelector('img')).toBeNull();
    expect(container.textContent).toBe('');
  });
});

describe('presence, not hue, carries the state at rest', () => {
  it('renders the etched ground only when pinned', () => {
    draw({ isPinned: true });
    expect(screen.queryByTestId('cdt-pin-ground')).not.toBeNull();
    cleanup();
    draw({ isPinned: false, visible: true });
    expect(screen.queryByTestId('cdt-pin-ground')).toBeNull();
  });

  it('marks its own visibility for the stylesheet rather than dimming a token', () => {
    draw({ isPinned: false, visible: false });
    const button = screen.getByRole('button');
    expect(button.getAttribute('data-visible')).toBe('false');
    expect(button.getAttribute('data-pinned')).toBe('false');
    expect(button.style.opacity).toBe('');
  });

  it('renders identically whether pinned by hover or by focus — one visible flag', () => {
    draw({ isPinned: false, visible: true });
    expect(screen.getByRole('button').getAttribute('data-visible')).toBe('true');
  });
});

describe('toggling', () => {
  it('calls back and stops the click reaching the card, whose body is Play', () => {
    // The card must be a real ancestor **inside the React tree**. React 18 delegates to the root
    // container, so a native listener attached to `button.parentElement` in a bare render sits on
    // that same container and fires whatever the handler does — `stopPropagation` does not stop
    // a listener on the element it is already at. Such a test fails against correct code and
    // would be "fixed" by deleting the guard the card actually needs.
    const onToggle = vi.fn();
    const onCardClick = vi.fn();
    render(
      <div data-testid="card-body" onClick={onCardClick}>
        <PinControl
          projectName="Aurora"
          isPinned={false}
          surface="card"
          visible
          onToggle={onToggle}
        />
      </div>,
    );
    fireEvent.click(screen.getByRole('button'));
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(onCardClick).not.toHaveBeenCalled();

    // …and the guard is real, not an artefact of the harness: a click on the card body itself
    // does reach it.
    fireEvent.click(screen.getByTestId('card-body'));
    expect(onCardClick).toHaveBeenCalledTimes(1);
  });

  it('renders from the prop, so the caller can flip its own projection immediately', () => {
    const { rerender } = render(
      <PinControl projectName="A" isPinned={false} surface="card" visible onToggle={() => {}} />,
    );
    expect(screen.getByRole('button').getAttribute('aria-pressed')).toBe('false');
    rerender(<PinControl projectName="A" isPinned surface="card" visible onToggle={() => {}} />);
    expect(screen.getByRole('button').getAttribute('aria-pressed')).toBe('true');
  });
});

describe('pinning moves nothing', () => {
  it('exposes no ordering, sectioning or aggregate surface of any kind', () => {
    // Structural, and it has to be: §7.8a's whole negative half is that `is_pinned` changes no
    // section list, no membership, no order, no SORT key, no header aggregate, no collapse state
    // and no attention chip. The module exporting exactly these two names is what a reviewer can
    // check; a probe of an undefined global, which is what this assertion first was, passes
    // against anything at all.
    expect(Object.keys(pinModule).sort()).toEqual(['PIN_ROTATION', 'PinControl']);
    expect(PIN_ROTATION).toBe('rotate(-45deg)');
  });
});

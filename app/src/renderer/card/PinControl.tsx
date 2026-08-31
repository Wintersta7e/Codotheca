import type { MouseEvent, ReactElement } from 'react';
import { pinControlName } from '../a11y/names';
import { type CardSurface, bandsFor } from './geometry';

/**
 * §7.8a. The mark is a shape, not a colour, and it does **not** go on the frame edge: that is
 * where §7.7a spends its two marks, and in phase 1 the unknown gap is on every card. Band 1 owns
 * it, right-anchored, inboard of the condition dot and below the hazard tape.
 *
 * §11.7's `Pinned` name is carried by `aria-pressed`, not by a second hidden string: in phase 1
 * the mark *is* the control, so a standalone name would name a component that never mounts.
 *
 * Nothing here orders, sections or aggregates. `is_pinned` changes no section list, no
 * membership, no order within a section, no SORT key, no header aggregate, no collapse state and
 * no attention chip; `is:pinned` matching it is the entire phase-1 consequence.
 */
export interface PinControlProps {
  readonly projectName: string;
  readonly isPinned: boolean;
  readonly surface: CardSurface;
  /** Hover (§7.8's `hov`) or focus (§11.7's focused project id). Never CSS `:hover`. */
  readonly visible: boolean;
  /**
   * `-1` on the grid, where §11.7 keeps one roving tabindex per grid and the pin is reached by
   * `P` while the cell holds focus. The project page has no grid and binds no `P`, so its hero
   * pin takes a real tab stop or it is a control nobody can reach.
   */
  readonly tabIndex?: 0 | -1;
  readonly onToggle: () => void;
}

export const PIN_ROTATION = 'rotate(-45deg)';

export function PinControl(props: PinControlProps): ReactElement {
  const pin = bandsFor(props.surface).pin;

  const onClick = (event: MouseEvent<HTMLButtonElement>): void => {
    // The card body is Play; the pin must not launch anything.
    event.stopPropagation();
    props.onToggle();
  };

  return (
    <button
      type="button"
      className="cdt-pin"
      tabIndex={props.tabIndex ?? -1}
      aria-pressed={props.isPinned}
      aria-label={pinControlName(props.projectName, props.isPinned)}
      data-pinned={props.isPinned ? 'true' : 'false'}
      data-visible={props.visible ? 'true' : 'false'}
      onClick={onClick}
      style={{
        position: 'absolute',
        width: `${String(pin.hit)}px`,
        height: `${String(pin.hit)}px`,
        right: `${String(pin.hitRight)}px`,
        top: `${String(pin.hitTop)}px`,
      }}
    >
      {props.isPinned ? (
        <span
          className="cdt-pin-ground"
          data-testid="cdt-pin-ground"
          style={{
            width: `${String(pin.box)}px`,
            height: `${String(pin.box)}px`,
            right: `${String(pin.right - pin.hitRight)}px`,
            top: `${String(pin.top - pin.hitTop)}px`,
          }}
        />
      ) : null}
      <span
        className="cdt-pin-silhouette"
        data-testid="cdt-pin-silhouette"
        style={{
          width: `${String(pin.box)}px`,
          height: `${String(pin.box)}px`,
          right: `${String(pin.right - pin.hitRight)}px`,
          top: `${String(pin.top - pin.hitTop)}px`,
          // A static transform, not a transition: `off` forbids transitions and animations,
          // not geometry, so the mark is identical at full, reduced and off.
          transform: PIN_ROTATION,
        }}
      >
        <span
          className="cdt-pin-bar"
          data-testid="cdt-pin-bar"
          style={{
            width: `${String(pin.barW)}px`,
            height: `${String(pin.barH)}px`,
            left: `${String((pin.box - pin.barW) / 2)}px`,
            top: `${String((pin.box - pin.barH) / 2 - pin.shaftH / 2)}px`,
          }}
        />
        <span
          className="cdt-pin-shaft"
          data-testid="cdt-pin-shaft"
          style={{
            width: `${String(pin.shaftW)}px`,
            height: `${String(pin.shaftH)}px`,
            left: `${String((pin.box - pin.shaftW) / 2)}px`,
            top: `${String((pin.box - pin.barH) / 2 + pin.barH / 2 - pin.shaftH / 2)}px`,
          }}
        />
      </span>
    </button>
  );
}

/**
 * §11.3a's drawer chrome, and the one place it is drawn: a 400px panel at the right edge of a
 * scrim. The settings drawer and §11.1's scan summary both sit in it, so its values stay in
 * `styles.ts` and its structure stays here — a second copy of either would drift.
 *
 * The backdrop is the only close-on-click target; clicks inside the panel do not bubble to it.
 * Keys pressed inside the panel stay inside it (§11.7: each context owns its keys exclusively):
 * the shelf listens on the window, so a key that bubbled out would act on the grid beneath —
 * `Enter` on a button here would open the selected project and `Esc` would close its Peek. The
 * palette's `Alt+Space` and the level flip listen in the capture phase and are not affected.
 */
import type { KeyboardEvent, ReactElement, ReactNode, RefObject } from 'react';
import type { ResolvedTier } from '../motion/tier.js';
import { SD, backdropAnimation, panelAnimation } from './styles.js';

export interface DrawerFrameProps {
  readonly tier: ResolvedTier;
  /** The dialog's accessible name. */
  readonly label: string;
  readonly backdropTestId: string;
  readonly onClose: () => void;
  /** The surface's own keys; whatever it does, the event goes no further than the panel. */
  readonly onKeyDown?: (event: KeyboardEvent<HTMLDivElement>) => void;
  readonly panelRef?: RefObject<HTMLDivElement | null>;
  readonly children: ReactNode;
}

export function DrawerFrame(props: DrawerFrameProps): ReactElement {
  const { tier, onKeyDown } = props;
  return (
    <div
      data-testid={props.backdropTestId}
      style={{ ...SD.backdrop, ...backdropAnimation(tier) }}
      onClick={props.onClose}
    >
      <div
        ref={props.panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={props.label}
        tabIndex={-1}
        style={{ ...SD.panel, ...panelAnimation(tier) }}
        onClick={(event) => {
          event.stopPropagation();
        }}
        onKeyDown={(event) => {
          onKeyDown?.(event);
          event.stopPropagation();
        }}
      >
        {props.children}
      </div>
    </div>
  );
}

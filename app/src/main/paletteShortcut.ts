import type { ShortcutState } from '../shared/channels.js';
import { ResidentShortcut, residentShortcutStatusText } from './shortcut.js';
import type { BindOutcome, ShortcutHost } from './shortcut.js';

export interface ResidentShortcutDeps {
  readonly host: ShortcutHost;
  /** Show and focus the window, creating it if the 30-minute hybrid destroyed it. */
  readonly showWindow: () => void;
  /** Send `IPC_OPEN_PALETTE` to the renderer. */
  readonly openPalette: () => void;
  /** R32: send `IPC_SHORTCUT_STATE` to the renderer. Called on every transition. */
  readonly publishShortcutState: (state: ShortcutState) => void;
}

export interface ResidentShortcutHandle {
  readonly apply: (chord: string | null) => BindOutcome;
  readonly statusText: () => string;
  readonly dispose: () => void;
}

/**
 * R32: `register` returns a boolean and never says *why* it failed (§8.6), so `{chord, registered}`
 * is the whole truth this process has. A refusal keeps the chord — dropping it would erase the one
 * thing §11.3a's row must show.
 */
export function shortcutStateOf(outcome: BindOutcome): ShortcutState {
  switch (outcome.kind) {
    case 'unbound':
      return { chord: null, registered: false };
    case 'bound':
      return { chord: outcome.chord, registered: true };
    case 'refused':
      return { chord: outcome.chord, registered: false };
  }
}

/**
 * §8.6: pressing the resident shortcut shows the window and opens the palette — one press with
 * two terminal events. The window comes first: the palette paints inside a window that exists.
 *
 * R32: this is also the only place that knows what the binding is doing, so it publishes every
 * transition — including the unbound state at install, so the drawer never has to guess what it
 * missed, and on dispose, so a quit does not leave a stale chord on screen.
 */
export function installResidentShortcut(deps: ResidentShortcutDeps): ResidentShortcutHandle {
  const shortcut = new ResidentShortcut(deps.host, () => {
    deps.showWindow();
    deps.openPalette();
  });
  const publish = (outcome: BindOutcome): BindOutcome => {
    deps.publishShortcutState(shortcutStateOf(outcome));
    return outcome;
  };
  publish(shortcut.outcome);
  return {
    apply: (chord) => publish(shortcut.bind(chord)),
    statusText: () => residentShortcutStatusText(shortcut.outcome),
    dispose: () => {
      shortcut.dispose();
      publish(shortcut.outcome);
    },
  };
}

/** §8.6: tray-icon activation is not that gesture. It shows the shelf and opens nothing. */
export function activateFromTray(deps: Pick<ResidentShortcutDeps, 'showWindow'>): void {
  deps.showWindow();
}

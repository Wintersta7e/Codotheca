/**
 * The IPC surface between the shell and the renderer. Imported by both ends, so a channel
 * name cannot drift between them.
 */
import type { ErrorCode, Outcome, Topic } from '../generated/protocol';

export const IPC_REQUEST = 'codotheca:request';
export const IPC_CORE_STATUS = 'codotheca:core-status';
export const IPC_EVENTS = 'codotheca:events';

/**
 * The shell tells the renderer to open quick switch. §8.6: pressing the resident show shortcut
 * shows the window and opens the palette; tray-icon activation shows the window and opens
 * nothing, so the tray path never sends on this channel.
 */
export const IPC_OPEN_PALETTE = 'codotheca:open-palette';

// R32: the channel and the frame live beside the class that publishes them. Plans 17 and 17b
// import both and declare neither; `src/shared` because the renderer cannot import `src/main`.
export const IPC_SHORTCUT_STATE = 'codotheca:shortcut-state';

/** §8.6: a chord that failed to register reads back with the chord and `registered: false`. */
export interface ShortcutState {
  readonly chord: string | null;
  readonly registered: boolean;
}

export interface RendererEvent {
  topic: Topic;
  event: string;
  data: unknown;
}

export interface BridgeError {
  code: ErrorCode;
  message: string;
  /** `null` — definitely did not take effect. `'unknown'` — may have (§2.2). */
  outcome: Outcome | null;
  retryable: boolean;
}

export type BridgeReply = { ok: true; value: unknown } | { ok: false; error: BridgeError };

export interface BridgeCall {
  name: string;
  args: unknown;
}

/**
 * §2.4: `locations.relocate` is privileged, so `isRendererCallable` refuses it on IPC_REQUEST.
 * It travels this channel instead, where the shell owns the native folder dialog and the
 * renderer supplies nothing but an opaque LocationId. A renderer-supplied string and a dialog
 * result are different trust categories, and only one of them may become a path.
 */
export const IPC_RELOCATE = 'codotheca:relocate';

export interface RelocateCall {
  locationId: number;
}

export type RelocateReply =
  | { kind: 'relocated'; location: unknown }
  | { kind: 'cancelled' }
  | { kind: 'failed'; error: BridgeError };

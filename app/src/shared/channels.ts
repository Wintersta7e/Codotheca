/**
 * The IPC surface between the shell and the renderer. Imported by both ends, so a channel
 * name cannot drift between them.
 */
import type { ErrorCode, Outcome, Topic } from '../generated/protocol';

export const IPC_REQUEST = 'codotheca:request';
export const IPC_CORE_STATUS = 'codotheca:core-status';
export const IPC_EVENTS = 'codotheca:events';

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

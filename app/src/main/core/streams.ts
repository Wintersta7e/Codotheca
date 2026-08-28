/**
 * Gap detection for one subscribed topic.
 *
 * The consumer half of §2.3. v1 dropped to a coalesced snapshot and had no rule for the old
 * deltas that then arrived behind it; this is that rule.
 */
import type { EventFrame, SnapshotFrame } from './wire';

export interface StreamState {
  epoch: number;
  lastSeq: number | null;
}

export type AcceptResult =
  | { kind: 'delivered'; seq: number }
  | { kind: 'snapshot'; throughSeq: number }
  | { kind: 'gap'; expected: number; got: number }
  | { kind: 'stale'; reason: 'epoch' | 'replayed' };

export function newStreamState(epoch: number): StreamState {
  return { epoch, lastSeq: null };
}

/**
 * Folds one frame into the stream state. A `gap` means the consumer must ask for a fresh
 * snapshot; the frame that produced it is deliberately not delivered.
 */
export function acceptFrame(state: StreamState, frame: EventFrame | SnapshotFrame): AcceptResult {
  if (frame.epoch !== state.epoch) return { kind: 'stale', reason: 'epoch' };
  if (frame.t === 'snapshot') {
    state.lastSeq = frame.through_seq;
    return { kind: 'snapshot', throughSeq: frame.through_seq };
  }
  if (state.lastSeq === null) {
    state.lastSeq = frame.seq;
    return { kind: 'delivered', seq: frame.seq };
  }
  const expected = state.lastSeq + 1;
  if (frame.seq <= state.lastSeq) return { kind: 'stale', reason: 'replayed' };
  if (frame.seq > expected) return { kind: 'gap', expected, got: frame.seq };
  state.lastSeq = frame.seq;
  return { kind: 'delivered', seq: frame.seq };
}

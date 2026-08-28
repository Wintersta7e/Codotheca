import { describe, expect, it } from 'vitest';
import { acceptFrame, newStreamState } from './streams';
import type { EventFrame, SnapshotFrame } from './wire';

const ev = (seq: number, epoch = 1): EventFrame => ({
  t: 'event',
  epoch,
  topic: 'projects',
  event: 'upserted',
  seq,
  data: {},
});
const snap = (throughSeq: number, epoch = 1): SnapshotFrame => ({
  t: 'snapshot',
  epoch,
  topic: 'projects',
  through_seq: throughSeq,
  data: {},
});

describe('stream gap detection', () => {
  it('delivers consecutive deltas', () => {
    const s = newStreamState(1);
    expect(acceptFrame(s, ev(1)).kind).toBe('delivered');
    expect(acceptFrame(s, ev(2)).kind).toBe('delivered');
  });

  it('reports a hole as a gap and does not deliver the frame', () => {
    const s = newStreamState(1);
    acceptFrame(s, ev(1));
    expect(acceptFrame(s, ev(4))).toEqual({ kind: 'gap', expected: 2, got: 4 });
    expect(s.lastSeq).toBe(1);
  });

  it('accepts only deltas past through_seq after a snapshot', () => {
    const s = newStreamState(1);
    acceptFrame(s, ev(1));
    expect(acceptFrame(s, snap(9)).kind).toBe('snapshot');
    expect(acceptFrame(s, ev(7))).toEqual({ kind: 'stale', reason: 'replayed' });
    expect(acceptFrame(s, ev(10)).kind).toBe('delivered');
  });

  it('calls a frame from a dead epoch stale, never a gap', () => {
    const s = newStreamState(2);
    expect(acceptFrame(s, ev(1, 1))).toEqual({ kind: 'stale', reason: 'epoch' });
  });
});

import { test, expect } from 'vitest';
import { INITIAL_FIRST_RUN, SETTLE_HOLD_MS, firstRunReducer, shouldRunFirstRun } from './phase';
import type { FirstRunState } from './phase';
import type { ScanRunId } from '../../generated/protocol';

const at = (phase: FirstRunState['phase']): FirstRunState => ({
  ...INITIAL_FIRST_RUN,
  phase,
  consented: true,
});

// §10.5: the reveal never replays. A scan run exists from the moment DIG is pressed.
test('first run is offered only when no scan has ever started', () => {
  expect(shouldRunFirstRun({ runId: null })).toBe(true);
  expect(shouldRunFirstRun({ runId: 7 as ScanRunId })).toBe(false);
});

// §10.1b: unticking row 1 is honoured — DIG goes inert.
test('the flow cannot leave the roots screen without consent', () => {
  const withheld = firstRunReducer(INITIAL_FIRST_RUN, { kind: 'consent', granted: false });
  expect(firstRunReducer(withheld, { kind: 'dig' }).phase).toBe('roots');
  const granted = firstRunReducer(INITIAL_FIRST_RUN, { kind: 'consent', granted: true });
  expect(firstRunReducer(granted, { kind: 'dig' }).phase).toBe('scanning');
});

// §10.3a: exactly one settle at walk completion, then 700 ms before the reveal takes the
// screen. The hold is a beat, not a fade, and the reveal must not pre-empt it.
test('the reveal waits out the settle hold', () => {
  const finished = firstRunReducer(at('scanning'), { kind: 'walk_finished', at: 1_000 });
  expect(finished.phase).toBe('scanning');
  expect(finished.walkFinishedAt).toBe(1_000);
  expect(firstRunReducer(finished, { kind: 'tick', at: 1_000 + SETTLE_HOLD_MS - 1 }).phase).toBe(
    'scanning',
  );
  expect(firstRunReducer(finished, { kind: 'tick', at: 1_000 + SETTLE_HOLD_MS }).phase).toBe(
    'reveal',
  );
});

// §10.3a: SKIP AHEAD is present in the first rendered frame and takes the user to the reveal
// immediately — the walk keeps running behind it.
test('skip ahead reaches the reveal without waiting for the walk', () => {
  expect(firstRunReducer(at('scanning'), { kind: 'skip_ahead' }).phase).toBe('reveal');
});

test('the reveal leads to the turn and the turn leads to the shelf, both ways out', () => {
  expect(firstRunReducer(at('reveal'), { kind: 'go_on' }).phase).toBe('turn');
  expect(firstRunReducer(at('turn'), { kind: 'show_me' }).phase).toBe('shelf');
  expect(firstRunReducer(at('turn'), { kind: 'not_now' }).phase).toBe('shelf');
});

test('nothing returns to a beat that has already played', () => {
  const shelf = at('shelf');
  for (const action of [
    { kind: 'dig' },
    { kind: 'skip_ahead' },
    { kind: 'walk_finished', at: 5 },
    { kind: 'go_on' },
  ] as const) {
    expect(firstRunReducer(shelf, action).phase).toBe('shelf');
  }
});

// §10.4a: a shelf with no projects never reaches the reveal and gets §11.1 instead.
test('an empty library skips the reveal and the turn entirely', () => {
  const finished = firstRunReducer(at('scanning'), { kind: 'walk_finished', at: 0 });
  const empty = firstRunReducer(finished, { kind: 'nothing_found' });
  expect(empty.phase).toBe('shelf');
});

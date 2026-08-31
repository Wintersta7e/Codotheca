import { describe, expect, it } from 'vitest';
import type { CollectionId } from '../../generated/protocol.js';
import { ARM_SUB_LINE, DISARM_MS, DISARMED, armedDeleteStep, isArmed } from './armedDelete.js';

const A = 1 as CollectionId;
const B = 2 as CollectionId;

describe('armedDeleteStep', () => {
  it('arms on the first press and commits nothing', () => {
    const step = armedDeleteStep(DISARMED, { type: 'press', id: A, nowMs: 1_000 });
    expect(step.commit).toBeNull();
    expect(isArmed(step.state, A)).toBe(true);
    expect(isArmed(step.state, B)).toBe(false);
  });

  it('commits on the second press of the same chip, and disarms itself', () => {
    const armed = armedDeleteStep(DISARMED, { type: 'press', id: A, nowMs: 1_000 }).state;
    const step = armedDeleteStep(armed, { type: 'press', id: A, nowMs: 1_400 });
    expect(step.commit).toBe(A);
    expect(step.state).toEqual(DISARMED);
  });

  // The clock is the rule, not the timer. A throttled background renderer can leave a stale
  // arm standing; a press that lands outside the window re-arms and never deletes.
  it('re-arms rather than committing when the second press lands after the window', () => {
    const armed = armedDeleteStep(DISARMED, { type: 'press', id: A, nowMs: 1_000 }).state;
    const step = armedDeleteStep(armed, { type: 'press', id: A, nowMs: 1_000 + DISARM_MS + 1 });
    expect(step.commit).toBeNull();
    expect(isArmed(step.state, A)).toBe(true);
  });

  it('accepts a press exactly on the boundary', () => {
    const armed = armedDeleteStep(DISARMED, { type: 'press', id: A, nowMs: 1_000 }).state;
    expect(armedDeleteStep(armed, { type: 'press', id: A, nowMs: 1_000 + DISARM_MS }).commit).toBe(
      A,
    );
  });

  it('arming a second chip disarms the first and never commits it', () => {
    const armed = armedDeleteStep(DISARMED, { type: 'press', id: A, nowMs: 1_000 }).state;
    const step = armedDeleteStep(armed, { type: 'press', id: B, nowMs: 1_100 });
    expect(step.commit).toBeNull();
    expect(isArmed(step.state, A)).toBe(false);
    expect(isArmed(step.state, B)).toBe(true);
  });

  it('Esc disarms without committing', () => {
    const armed = armedDeleteStep(DISARMED, { type: 'press', id: A, nowMs: 1_000 }).state;
    const step = armedDeleteStep(armed, { type: 'disarm' });
    expect(step.commit).toBeNull();
    expect(step.state).toEqual(DISARMED);
  });

  it('a sweep disarms only once the window has actually passed', () => {
    const armed = armedDeleteStep(DISARMED, { type: 'press', id: A, nowMs: 1_000 }).state;
    expect(isArmed(armedDeleteStep(armed, { type: 'sweep', nowMs: 4_000 }).state, A)).toBe(true);
    const swept = armedDeleteStep(armed, { type: 'sweep', nowMs: 1_000 + DISARM_MS + 1 });
    expect(swept.state).toEqual(DISARMED);
    expect(swept.commit).toBeNull();
  });

  // A sweep over an already-disarmed state is a no-op, not a commit. The scheduled callback can
  // land after an `Esc` has already cleared the arm.
  it('a sweep over the disarmed state commits nothing', () => {
    const step = armedDeleteStep(DISARMED, { type: 'sweep', nowMs: 9_000 });
    expect(step.state).toEqual(DISARMED);
    expect(step.commit).toBeNull();
  });

  // Criterion 50: the arm is legible from wording alone, with no colour and no motion.
  it('states in words what deletion does not touch', () => {
    expect(ARM_SUB_LINE).toBe('PRESS AGAIN TO DELETE · THE PROJECTS STAY');
    expect(DISARM_MS).toBe(6_000);
  });
});

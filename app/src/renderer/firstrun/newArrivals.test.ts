import { describe, expect, it } from 'vitest';
import type { ProjectId } from '../../generated/protocol';
import { type NewArrivalRow, isNewArrival } from './newArrivals';

const FIRST_RUN = 1_700_000_000;
const id = (n: number): ProjectId => n as unknown as ProjectId;

const row = (over: Partial<NewArrivalRow> = {}): NewArrivalRow => ({
  id: id(1),
  createdAt: FIRST_RUN + 86_400,
  acknowledgedAt: null,
  ...over,
});

describe('§10.5a: created after first run and never opened', () => {
  it('is new when both halves hold', () => {
    expect(isNewArrival(row(), FIRST_RUN)).toBe(true);
  });

  it('is not new once it has been acknowledged', () => {
    expect(isNewArrival(row({ acknowledgedAt: FIRST_RUN + 90_000 }), FIRST_RUN)).toBe(false);
  });

  it('is not new when it predates the boundary', () => {
    expect(isNewArrival(row({ createdAt: FIRST_RUN - 1 }), FIRST_RUN)).toBe(false);
  });

  it('reads the boundary as exclusive, so a project indexed by first run is not new', () => {
    // Keyed on `created_at` alone every project is new on day one; the boundary is what stops
    // the entire first scan arriving as a wall of chips.
    expect(isNewArrival(row({ createdAt: FIRST_RUN }), FIRST_RUN)).toBe(false);
  });
});

describe('while first run has not completed', () => {
  it('is never new, because nothing is new relative to what the user had', () => {
    expect(isNewArrival(row(), null)).toBe(false);
    expect(isNewArrival(row({ createdAt: FIRST_RUN + 1_000_000 }), null)).toBe(false);
  });
});

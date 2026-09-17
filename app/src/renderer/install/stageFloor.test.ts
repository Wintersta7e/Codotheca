import { describe, expect, it } from 'vitest';

import type { InstallStage, InstallStageKind } from '../../generated/protocol.js';
import { pacedStages, stageBytes, stageFigure, STAGE_FLOOR_MS } from './stageFloor.js';

function stage(
  kind: InstallStageKind,
  done: number | null = null,
  total: number | null = null,
  bytes: number | null = null,
): InstallStage {
  return { runId: 1, stage: kind, done, total, bytes } as InstallStage;
}

const FULL: InstallStage[] = [
  stage('plans'),
  stage('enumerating', 3007),
  stage('receiving', 2196, 3007, 2_243_952),
  stage('assembling', 900, 900),
  stage('cladding', 41, 41),
  stage('settled'),
];

describe('the pacer', () => {
  it('never completes faster than the floor', () => {
    expect(pacedStages(FULL, 0)).toHaveLength(1);
    expect(pacedStages(FULL, STAGE_FLOOR_MS - 1).length).toBeLessThan(FULL.length);
    expect(pacedStages(FULL, STAGE_FLOOR_MS)).toHaveLength(FULL.length);
  });

  it('compresses a 300 ms clone rather than flashing six stages', () => {
    // The whole transcript arrives at once; at 300 ms only part of it may be on screen.
    const shown = pacedStages(FULL, 300);
    expect(shown.length).toBeGreaterThan(0);
    expect(shown.length).toBeLessThan(FULL.length);
  });

  it('never skips a stage: what is shown is always a prefix of what was observed', () => {
    for (let t = 0; t <= STAGE_FLOOR_MS + 200; t += 37) {
      const shown = pacedStages(FULL, t);
      expect(shown).toEqual(FULL.slice(0, shown.length).map((s) => s.stage));
    }
  });

  it('only grows, so a stage once shown is never taken away', () => {
    let previous = 0;
    for (let t = 0; t <= STAGE_FLOOR_MS + 200; t += 13) {
      const shown = pacedStages(FULL, t).length;
      expect(shown).toBeGreaterThanOrEqual(previous);
      previous = shown;
    }
  });

  it('renders nothing for a run that has observed nothing', () => {
    expect(pacedStages([], 5_000)).toEqual([]);
  });
});

describe('the figure', () => {
  it('is a bare count when the phase has no denominator', () => {
    expect(stageFigure(stage('enumerating', 3007))).toBe('3,007');
  });

  it('is <done> of <total> when it has one', () => {
    expect(stageFigure(stage('receiving', 2196, 3007))).toBe('2,196 of 3,007');
  });

  it('is nothing at all when the phase reported no count — never a zero', () => {
    expect(stageFigure(stage('plans'))).toBeNull();
  });

  it('carries no percentage anywhere', () => {
    for (const s of FULL) {
      expect(stageFigure(s) ?? '').not.toContain('%');
    }
  });
});

describe('the byte figure', () => {
  it('is absent where git reported none', () => {
    expect(stageBytes(stage('assembling', 1, 1))).toBeNull();
  });

  it('reads in MB above a tenth of one', () => {
    expect(stageBytes(stage('receiving', 1, 2, 2_243_952))).toBe('2.1 MB');
  });
});

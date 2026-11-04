/**
 * §43.2's three tables as pure models. Each enumerates its variants from the **tracked** schema —
 * never from `src/generated`, which is gitignored — renders every one, prints covered over total,
 * and fails at zero or on a variant with no rendering (§49.2 rule 12). An unknown is drawn as
 * U+2014 and named in words; it is never a zero and never an empty ring.
 */
import { describe, expect, it, test } from 'vitest';
// `?raw` rather than node:fs: the renderer project carries no Node types by design.
import schemaRaw from '../../../../protocol/schema/protocol.json?raw';
import type {
  LevelReading,
  LevelState,
  RhythmCoverage,
  RhythmDay,
  RhythmState,
  RingsState,
  RingStatus,
} from '../../generated/protocol';
import {
  DAY_ONE_SENTENCE,
  LEVEL_NOT_COMPUTED,
  RHYTHM_PARTIAL,
  RING_LABELS,
  RING_NOT_APPLICABLE,
  TEND_NOTHING_TO_TEND,
  UNKNOWN,
  XP_LINE_COMPUTED,
  XP_LINE_FLOORED,
  XP_LINE_PARTIAL,
} from './copy';
import { levelBlockModel } from './levelBlock';
import { rhythmModel } from './rhythm';
import { ringsModel } from './rings';

function variantsOf(type: string): string[] {
  expect(schemaRaw.length, 'protocol.json read as an empty string').toBeGreaterThan(0);
  const raw = JSON.parse(schemaRaw) as { types?: Record<string, { variants?: string[] }> };
  return raw.types?.[type]?.variants ?? [];
}

const figures = { level: 7, xp: 1_300, levelAt: 1_000, nextLevelAt: 1_600, floored: false };

const reading = (state: LevelState, over: Partial<LevelReading> = {}): LevelReading => ({
  state,
  figures: state === 'notComputed' ? null : figures,
  pendingProjects: state === 'partial' ? 3 : 0,
  ...over,
});

const rhythm = (over: Partial<RhythmState> = {}): RhythmState => ({
  days: ['active', 'inactive', 'pending', 'future', 'future', 'future', 'future'],
  activeDays: 1,
  target: 4,
  targetBasis: 'derived',
  runWeeks: 2,
  bestWeeks: 6,
  gitCoverage: 'complete',
  ...over,
});

const ringsWith = (status: RingStatus, over: Partial<RingsState> = {}): RingsState => ({
  today: '2026-11-04',
  weekStart: '2026-11-02',
  waived: false,
  focus: { creditedSeconds: 1_380, targetSeconds: 2_700, status },
  ship: { status, pushedOn: null, observable: 2, unobservable: 0 },
  tend: { status, excluded: [], unobservableProjects: 0 },
  rhythm: rhythm(),
  ...over,
});

/** Renders each variant through `render`, prints the tally, and fails on zero or on a gap. */
function enumerate(type: string, render: (variant: string) => void): void {
  const variants = variantsOf(type);
  let covered = 0;
  for (const variant of variants) {
    expect(() => {
      render(variant);
    }, `${type}.${variant} has no rendering`).not.toThrow();
    covered += 1;
  }
  console.warn(`bandModels: ${type} covered ${String(covered)}/${String(variants.length)}`);
  expect(variants.length, `the schema declares no ${type} variant`).toBeGreaterThan(0);
  expect(covered).toBe(variants.length);
}

test('AC-P4-43-2 each band table renders every generated variant and never an unknown as zero', () => {
  enumerate('LevelState', (variant) => {
    const model = levelBlockModel(reading(variant as LevelState), 0.5);
    expect(model.number.length).toBeGreaterThan(0);
    expect(model.line.length).toBeGreaterThan(0);
  });
  enumerate('RingStatus', (variant) => {
    const model = ringsModel(ringsWith(variant as RingStatus));
    if (model.waived) throw new Error('a ring status drew the day-one block');
    expect(model.rings).toHaveLength(3);
    for (const ring of model.rings) expect(ring.name.length).toBeGreaterThan(0);
  });
  enumerate('RhythmDay', (variant) => {
    const model = rhythmModel(rhythm({ days: [variant as RhythmDay] }));
    expect(model.cells[0]?.name.length).toBeGreaterThan(0);
  });
  enumerate('RhythmCoverage', (variant) => {
    rhythmModel(rhythm({ gitCoverage: variant as RhythmCoverage }));
  });

  // notComputed: U+2014 and NOT COMPUTED, no bar and no track — never 0, never 1.
  const unknown = levelBlockModel(reading('notComputed'), 0.5);
  expect(unknown.number).toBe(UNKNOWN);
  expect(unknown.unknown).toBe(true);
  expect(unknown.line).toBe(LEVEL_NOT_COMPUTED);
  expect(unknown.bar).toBeNull();
  expect(unknown.number).not.toMatch(/[0-9]/u);
  expect(levelBlockModel(null, null).number).toBe(UNKNOWN);

  // partial names its pending projects; computed names the XP to the next level only.
  const partial = levelBlockModel(reading('partial'), 0.5);
  expect(partial.number).toBe('7');
  expect(partial.line).toBe(XP_LINE_PARTIAL(300, 3));
  expect(partial.bar).toBe(0.5);
  const computed = levelBlockModel(reading('computed'), 0.5);
  expect(computed.line).toBe(XP_LINE_COMPUTED(300));
  expect(computed.bar).toBe(0.5);

  // floored, either state: the level, the floor's line, no bar.
  for (const state of ['partial', 'computed'] as const) {
    const floored = levelBlockModel(
      reading(state, { figures: { ...figures, xp: 900, floored: true } }),
      0.5,
    );
    expect(floored.number).toBe('7');
    expect(floored.line).toBe(XP_LINE_FLOORED);
    expect(floored.bar).toBeNull();
  }

  // A notComputed ring is the --unknown glyph with U+2014 — never an empty ring.
  const rings = ringsModel(ringsWith('notComputed'));
  if (rings.waived) throw new Error('unreachable');
  for (const ring of rings.rings) {
    expect(ring.fill).toBe('unknown');
    expect(ring.centre).toBe(UNKNOWN);
    expect(ring.name).not.toMatch(/\b0\b/u);
  }
});

describe('the three tables, row by row', () => {
  it('fails on a variant it has no rendering for, so a new schema variant cannot ship blank', () => {
    expect(() => levelBlockModel(reading('bogus' as LevelState), 0.5)).toThrow();
    expect(() => ringsModel(ringsWith('bogus' as RingStatus))).toThrow();
    expect(() => rhythmModel(rhythm({ days: ['bogus' as RhythmDay] }))).toThrow();
    expect(() => rhythmModel(rhythm({ gitCoverage: 'bogus' as RhythmCoverage }))).toThrow();
  });

  it('draws a partial or computed reading that carries no figures as unknown, never a number', () => {
    const model = levelBlockModel(reading('computed', { figures: null }), 0.5);
    expect(model.number).toBe(UNKNOWN);
    expect(model.bar).toBeNull();
  });

  it('draws Focus open at its capped fraction and a binary open as an empty, measured track', () => {
    const open = ringsModel(ringsWith('open'));
    if (open.waived) throw new Error('unreachable');
    const [focus, ship, tend] = open.rings;
    expect(focus?.label).toBe(RING_LABELS.focus);
    expect(focus?.fill).toBe('arc');
    expect(focus?.fraction).toBeCloseTo(1_380 / 2_700);
    expect(focus?.centre).toBe('23m');
    expect(ship?.label).toBe(RING_LABELS.ship);
    expect(ship?.fill).toBe('arc');
    expect(ship?.fraction).toBe(0);
    expect(tend?.label).toBe(RING_LABELS.tend);
    const over = ringsModel(
      ringsWith('closed', {
        focus: { creditedSeconds: 9_000, targetSeconds: 2_700, status: 'closed' },
      }),
    );
    if (over.waived) throw new Error('unreachable');
    expect(over.rings[0]?.fraction).toBe(1);
    expect(over.rings[0]?.closed).toBe(true);
  });

  it('draws nothingToTend full with its own sub-line, and notApplicable as track and mark', () => {
    const tend = ringsModel(ringsWith('nothingToTend'));
    if (tend.waived) throw new Error('unreachable');
    expect(tend.rings[2]?.fraction).toBe(1);
    expect(tend.rings[2]?.sub).toBe(TEND_NOTHING_TO_TEND);
    const na = ringsModel(ringsWith('notApplicable'));
    if (na.waived) throw new Error('unreachable');
    expect(na.rings[1]?.fill).toBe('track');
    expect(na.rings[1]?.centre).toBe(RING_NOT_APPLICABLE);
  });

  it('waives day one with no glyph, the three labels and exactly one sentence', () => {
    const model = ringsModel(
      ringsWith('open', { waived: true, focus: null, ship: null, tend: null }),
    );
    if (!model.waived) throw new Error('the waived state drew glyphs');
    expect(model.labels).toEqual([RING_LABELS.focus, RING_LABELS.ship, RING_LABELS.tend]);
    expect(model.sentence).toBe(DAY_ONE_SENTENCE);
    expect('rings' in model).toBe(false);
  });

  it('qualifies the rhythm line while git coverage is partial, and draws seven named cells', () => {
    expect(rhythmModel(rhythm()).partial).toBeNull();
    expect(rhythmModel(rhythm({ gitCoverage: 'partial' })).partial).toBe(RHYTHM_PARTIAL);
    const week = rhythmModel(
      rhythm({
        days: ['active', 'inactive', 'pending', 'future', 'waived', 'notComputed', 'future'],
      }),
    );
    expect(week.cells.map((c) => c.letter).join('')).toBe('MTWTFSS');
    expect(new Set(week.cells.map((c) => c.name)).size).toBe(7);
    // A waived day is never drawn as inactive.
    expect(week.cells[4]?.day).toBe('waived');
    expect(week.cells[4]?.name).not.toBe(week.cells[1]?.name.replace('Tuesday', 'Friday'));
  });

  it('renders an unset target and an unknown best rhythm as U+2014, never 0', () => {
    const line = rhythmModel(rhythm({ target: null, bestWeeks: null, activeDays: 2 })).line;
    expect(line).toContain(UNKNOWN);
    expect(line).not.toMatch(/\b0\b/u);
  });

  it('draws the active-day count as U+2014 while any day of the week is not computed, never 0', () => {
    const fresh = rhythmModel(
      rhythm({
        days: ['notComputed', 'notComputed', 'notComputed', 'future', 'future', 'future', 'future'],
        activeDays: 0,
      }),
    ).line;
    expect(fresh.match(/—/gu)).toHaveLength(1);
    expect(fresh).not.toMatch(/\b0\b/u);
    // One unread day is enough: a count over a week with a hole in it is not a count.
    const holed = rhythmModel(
      rhythm({
        days: ['active', 'notComputed', 'pending', 'future', 'future', 'future', 'future'],
        activeDays: 1,
      }),
    ).line;
    expect(holed.match(/—/gu)).toHaveLength(1);
    expect(holed).not.toMatch(/\b1\b/u);
    // A week read in full that holds no active day is a real zero.
    const idle = rhythmModel(
      rhythm({
        days: ['inactive', 'inactive', 'pending', 'future', 'future', 'future', 'future'],
        activeDays: 0,
      }),
    ).line;
    expect(idle).toMatch(/\b0\b/u);
    expect(idle).not.toContain(UNKNOWN);
  });
});

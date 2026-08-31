import { describe, expect, it } from 'vitest';
import { CHIP_CAP, CHIP_TYPE, statusChips } from '../card/chips.js';
import type { ChipRow } from '../card/chips.js';
import { LIST_CHIP_COLUMN_PX, LIST_CHIP_TYPE, listChipsLabel, listRowChips } from './chips.js';

const NOW = 1_770_000_000;
const row = (over: Partial<ChipRow> = {}): ChipRow =>
  ({
    id: 1,
    interruptedOp: null,
    refstateObservedAt: NOW,
    isDirty: null,
    worktreeObservedAt: null,
    ahead: null,
    behind: null,
    fetchHeadAt: null,
    createdAt: 0,
    acknowledgedAt: 0,
    ...over,
  }) as ChipRow;

describe("the list column's chips", () => {
  it("takes §8.0c's geometry, which is not the card's", () => {
    expect(LIST_CHIP_COLUMN_PX).toBe(128);
    expect(LIST_CHIP_TYPE.tracking).toBe('.1em');
    expect(LIST_CHIP_TYPE.padding).toBe('2px 4px');
    expect(LIST_CHIP_TYPE.tracking).not.toBe(CHIP_TYPE.tracking);
  });

  it("derives nothing of its own — the set is the card module's, reversed for a right-aligned column", () => {
    const chips = listRowChips(
      row({ interruptedOp: 'merge', isDirty: true, worktreeObservedAt: NOW }),
      NOW,
      0,
    );
    expect(chips.at(-1)?.id).toBe('interrupted');
  });

  it('announces every chip, including any the 128px column clips', () => {
    const chips = listRowChips(
      row({ interruptedOp: 'merge', isDirty: true, worktreeObservedAt: NOW }),
      NOW,
      0,
    );
    const label = listChipsLabel(chips);
    for (const chip of chips) expect(label).toContain(chip.accessibleName);
  });

  it('says nothing when there is nothing to flag', () => {
    expect(listChipsLabel(listRowChips(row(), NOW, 0))).toBe('');
  });

  it("is exactly the card module's set, reversed and nothing else", () => {
    const input = row({
      interruptedOp: 'merge',
      isDirty: true,
      worktreeObservedAt: NOW,
      ahead: 2,
      behind: 3,
      fetchHeadAt: NOW,
    });
    expect(listRowChips(input, NOW, 0)).toEqual([...statusChips(input, NOW, 0)].reverse());
  });

  it("keeps the card module's cap without widening or narrowing it", () => {
    const input = row({
      interruptedOp: 'merge',
      isDirty: true,
      worktreeObservedAt: NOW,
      ahead: 2,
      behind: 3,
      fetchHeadAt: NOW,
      createdAt: NOW,
      acknowledgedAt: null,
    });
    const cardChips = statusChips(input, NOW, NOW - 1);
    expect(cardChips.length).toBeGreaterThan(CHIP_CAP);
    expect(listRowChips(input, NOW, NOW - 1)).toHaveLength(cardChips.length);
  });

  it("joins every accessible name with ', '", () => {
    const input = row({
      interruptedOp: 'merge',
      isDirty: true,
      worktreeObservedAt: NOW,
      ahead: 2,
    });
    const chips = statusChips(input, NOW, 0);
    expect(listChipsLabel(chips)).toBe(chips.map((chip) => chip.accessibleName).join(', '));
  });
});

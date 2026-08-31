import { describe, expect, it } from 'vitest';
import type { ProjectId } from '../../generated/protocol';
import { CHIP_CAP, CHIP_TYPE, type ChipRow, statusChips } from './chips';

const NOW = 1_700_000_000;
const FIRST_RUN = NOW - 90 * 86_400;

const row = (over: Partial<ChipRow> = {}): ChipRow => ({
  id: 1 as unknown as ProjectId,
  interruptedOp: null,
  refstateObservedAt: NOW - 60,
  isDirty: null,
  worktreeObservedAt: null,
  ahead: null,
  behind: null,
  fetchHeadAt: null,
  createdAt: FIRST_RUN - 86_400,
  acknowledgedAt: null,
  ...over,
});

const ids = (over: Partial<ChipRow> = {}): string[] =>
  statusChips(row(over), NOW, FIRST_RUN).map((chip) => chip.id);

describe('the four status chips and their order', () => {
  it('orders interrupted, uncommitted, unpushed, behind', () => {
    expect(
      ids({
        interruptedOp: 'rebase',
        isDirty: true,
        worktreeObservedAt: NOW - 60,
        ahead: 2,
        behind: 11,
        fetchHeadAt: NOW - 6 * 86_400,
      }),
    ).toEqual(['interrupted', 'uncommitted', 'unpushed', 'behind']);
  });

  it('caps the status chips at four', () => {
    expect(CHIP_CAP).toBe(4);
    expect(
      ids({
        interruptedOp: 'merge',
        isDirty: true,
        worktreeObservedAt: NOW - 60,
        ahead: 2,
        behind: 3,
        fetchHeadAt: NOW - 60,
      }),
    ).toHaveLength(4);
  });

  it('draws the interrupt chip on --interrupt over --surface-0, never a hex', () => {
    const chip = statusChips(row({ interruptedOp: 'merge' }), NOW, FIRST_RUN)[0];
    expect(chip?.text).toBe('INTERRUPTED');
    expect(chip?.fillToken).toBe('interrupt');
    expect(chip?.inkToken).toBe('surface-0');
  });

  it('draws the two accent chips on --sig over --sig-ink', () => {
    const dirty = statusChips(row({ isDirty: true, worktreeObservedAt: NOW }), NOW, FIRST_RUN)[0];
    expect(dirty?.text).toBe('UNCOMMITTED');
    expect(dirty?.fillToken).toBe('sig');
    expect(dirty?.inkToken).toBe('sig-ink');
    const unpushed = statusChips(row({ ahead: 3 }), NOW, FIRST_RUN)[0];
    expect(unpushed?.text).toBe('UNPUSHED');
    expect(unpushed?.fillToken).toBe('sig');
  });

  it('sets the chip type once, at the 7px floor with its tracking', () => {
    expect(CHIP_TYPE).toEqual({ fontPx: 7, weight: 700, tracking: '.12em', padding: '2px 5px' });
  });
});

describe('a flag for zero is furniture', () => {
  it('omits UNCOMMITTED when is_dirty is NULL, and draws it only when true', () => {
    expect(ids({ isDirty: null })).not.toContain('uncommitted');
    expect(ids({ isDirty: false })).not.toContain('uncommitted');
    expect(ids({ isDirty: true, worktreeObservedAt: NOW })).toContain('uncommitted');
  });

  it('omits UNPUSHED when ahead is NULL or zero', () => {
    expect(ids({ ahead: null })).not.toContain('unpushed');
    expect(ids({ ahead: 0 })).not.toContain('unpushed');
    expect(ids({ ahead: 1 })).toContain('unpushed');
  });

  it('omits BEHIND entirely with no recorded fetch — never BEHIND 0', () => {
    expect(ids({ behind: 4, fetchHeadAt: null })).not.toContain('behind');
    expect(ids({ behind: 0, fetchHeadAt: NOW - 60 })).not.toContain('behind');
    expect(ids({ behind: null, fetchHeadAt: NOW - 60 })).not.toContain('behind');
    expect(ids({ behind: 4, fetchHeadAt: NOW - 60 })).toContain('behind');
  });

  it('carries the fetch age in the chip text, because §3.3 contains no fetch', () => {
    const chip = statusChips(row({ behind: 11, fetchHeadAt: NOW - 6 * 86_400 }), NOW, FIRST_RUN)[0];
    expect(chip?.text).toMatch(/^BEHIND 11 · /);
    expect(chip?.text).not.toBe('BEHIND 11');
  });
});

describe('NEW sits outside the cap and renders last', () => {
  const crowded: Partial<ChipRow> = {
    interruptedOp: 'merge',
    isDirty: true,
    worktreeObservedAt: NOW - 60,
    ahead: 2,
    behind: 3,
    fetchHeadAt: NOW - 60,
    createdAt: FIRST_RUN + 86_400,
    acknowledgedAt: null,
  };

  it('survives four git flags, which is the whole reason it is outside the cap', () => {
    const list = ids(crowded);
    expect(list).toHaveLength(5);
    expect(list.at(-1)).toBe('new');
    expect(list).toContain('interrupted');
  });

  it('needs a project created after first run and never opened', () => {
    expect(ids({ createdAt: FIRST_RUN + 86_400, acknowledgedAt: null })).toContain('new');
    expect(ids({ createdAt: FIRST_RUN + 86_400, acknowledgedAt: NOW })).not.toContain('new');
    expect(ids({ createdAt: FIRST_RUN - 86_400, acknowledgedAt: null })).not.toContain('new');
  });

  it('draws nothing while first run has not completed', () => {
    const list = statusChips(row({ createdAt: NOW, acknowledgedAt: null }), NOW, null);
    expect(list.map((chip) => chip.id)).not.toContain('new');
  });

  it('takes the system accent, not the card jewel — novelty is library-wide', () => {
    const chip = statusChips(row({ createdAt: FIRST_RUN + 86_400 }), NOW, FIRST_RUN).at(-1);
    expect(chip?.text).toBe('NEW');
    expect(chip?.fillToken).toBe('sig');
    expect(chip?.inkToken).toBe('sig-ink');
  });
});

describe('accessible names pair the chip with its observation time', () => {
  it('names uncommitted work with the worktree observation', () => {
    const chip = statusChips(
      row({ isDirty: true, worktreeObservedAt: NOW - 3600 }),
      NOW,
      FIRST_RUN,
    )[0];
    expect(chip?.accessibleName).toMatch(/^Uncommitted changes as of /);
  });

  it('names unpushed commits and an interrupted operation from the refstate observation', () => {
    expect(statusChips(row({ ahead: 2 }), NOW, FIRST_RUN)[0]?.accessibleName).toMatch(
      /^Unpushed commits as of /,
    );
    expect(
      statusChips(row({ interruptedOp: 'rebase' }), NOW, FIRST_RUN)[0]?.accessibleName,
    ).toMatch(/^Interrupted rebase as of /);
  });

  it('names BEHIND with its count and the fetch age', () => {
    const chip = statusChips(row({ behind: 11, fetchHeadAt: NOW - 6 * 86_400 }), NOW, FIRST_RUN)[0];
    expect(chip?.accessibleName).toMatch(/^Behind by 11 as of /);
  });

  it('names NEW with no observation time, because nothing observed it', () => {
    const chip = statusChips(row({ createdAt: FIRST_RUN + 86_400 }), NOW, FIRST_RUN).at(-1);
    expect(chip?.accessibleName).toBe('New');
  });

  it('falls back to the bare sentence when the observation time is missing', () => {
    const chip = statusChips(row({ isDirty: true, worktreeObservedAt: null }), NOW, FIRST_RUN)[0];
    expect(chip?.accessibleName).toBe('Uncommitted changes');
  });
});

describe('the chips phase 1 does not have', () => {
  it('emits no shield, no CI chip and no PINNED chip', () => {
    const text = statusChips(
      row({
        interruptedOp: 'merge',
        isDirty: true,
        worktreeObservedAt: NOW,
        ahead: 1,
        behind: 1,
        fetchHeadAt: NOW,
        createdAt: FIRST_RUN + 1,
      }),
      NOW,
      FIRST_RUN,
    )
      .map((chip) => chip.text)
      .join(' ');
    for (const banned of ['LICENSE', 'BUILD', 'RELEASE', '★', 'CI RED', 'PINNED', 'COMPLETION']) {
      expect(text).not.toContain(banned);
    }
  });
});

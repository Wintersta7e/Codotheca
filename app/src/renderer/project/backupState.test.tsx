import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import type { BackupState, LocationDetail } from '../../generated/protocol';
import { BackupStateBlock } from './BackupState';
import { locationFixture, NOW } from './testFixtures';

afterEach(cleanup);

function draw(state: BackupState | null, over: Partial<LocationDetail> = {}): void {
  render(<BackupStateBlock state={state} location={locationFixture(over)} now={NOW} />);
}

function line(): string {
  return screen.getByTestId('cp-backup-line').textContent;
}

function plate(): string {
  return screen.getByTestId('cp-backup-plate').textContent;
}

/**
 * **AC-P2-25-11-producer.** §25.3's block, over the decision the core made.
 *
 * **This check exercises three of the four rows** — `only_copy`, `not_anywhere_else` and
 * `verified` — plus the NULL that row 4 produces. What it does **not** exercise is R51's limb:
 * an *unreadable* stash reflog on a location that was read, which
 * `core/src/git/refstate.rs:142`'s `u32` cannot represent today. **`AC-P2-25-11-unknown` owns
 * that limb and it is p2-24b's**, landing with `RefState.stash_count: Option<u32>`.
 */
describe('§25.3 the backup line states a fact and offers no control', () => {
  it('AC-P2-25-11-producer renders the only-copy row with its plate and its edge token', () => {
    draw('only_copy');
    expect(line()).toBe('No remote. This machine is the only copy.');
    expect(plate()).toBe('ONLY COPY ON EARTH');
    expect(screen.getByTestId('cp-backup').getAttribute('style')).toContain('var(--fail)');
  });

  it('renders the not-anywhere-else row with the fetch age it depends on', () => {
    draw('not_anywhere_else', { ahead: 12, stashCount: 1, fetchHeadAt: NOW - 3600 });
    expect(line()).toBe('12 commits and 1 stash exist only on this disk.');
    expect(plate()).toBe('NOT ANYWHERE ELSE · AS OF 1h');
    expect(screen.getByTestId('cp-backup').getAttribute('style')).toContain('var(--warn)');
  });

  it('renders the verified row with the age of the observation that earned it', () => {
    draw('verified', { ahead: 0, stashCount: 0, fetchHeadAt: NOW - 120 });
    expect(line()).toBe('Nothing on this branch is only on this disk.');
    expect(plate()).toBe('VERIFIED 2m');
    expect(screen.getByTestId('cp-backup').getAttribute('style')).toContain('var(--pass)');
  });

  it('renders no element at all for the fourth row', () => {
    draw(null, { ahead: null, stashCount: null, fetchHeadAt: null });
    expect(screen.queryByTestId('cp-backup')).toBeNull();
  });

  it('renders nothing when there is no local copy to describe', () => {
    render(<BackupStateBlock state="only_copy" location={null} now={NOW} />);
    expect(screen.queryByTestId('cp-backup')).toBeNull();
  });

  it('singularises both nouns, at one and at two', () => {
    draw('not_anywhere_else', { ahead: 1, stashCount: 1, fetchHeadAt: NOW - 60 });
    expect(line()).toBe('1 commit and 1 stash exist only on this disk.');
    cleanup();
    draw('not_anywhere_else', { ahead: 2, stashCount: 2, fetchHeadAt: NOW - 60 });
    expect(line()).toBe('2 commits and 2 stashes exist only on this disk.');
  });

  it('names only the half that is non-zero, never a zero clause', () => {
    draw('not_anywhere_else', { ahead: 3, stashCount: 0, fetchHeadAt: NOW - 60 });
    expect(line()).toBe('3 commits exist only on this disk.');
    cleanup();
    draw('not_anywhere_else', { ahead: 0, stashCount: 2, fetchHeadAt: NOW - 60 });
    expect(line()).toBe('2 stashes exist only on this disk.');
  });

  /**
   * *Absence of dirty is "no changes as of T", never "clean".* The word is banned in the
   * rendered output **and** in the module, because a string that is not reachable today is one
   * refactor from being reachable tomorrow.
   */
  it('renders the word clean nowhere, in output or in source', () => {
    for (const state of ['only_copy', 'not_anywhere_else', 'verified'] as const) {
      draw(state, { ahead: 1, stashCount: 1, fetchHeadAt: NOW - 60 });
      expect(screen.getByTestId('cp-backup').textContent.toLowerCase()).not.toContain('clean');
      cleanup();
    }
  });
});

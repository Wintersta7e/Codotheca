import { describe, expect, it } from 'vitest';
import { rowFixture } from '../project/testFixtures';
import { neverSucceeded } from './errorKind';

describe('the never-succeeded state', () => {
  it('is an error with no observation ever recorded', () => {
    expect(
      neverSucceeded(
        rowFixture({
          errorKind: 'PERMISSION_DENIED',
          refstateObservedAt: null,
          worktreeObservedAt: null,
        }),
      ),
    ).toBe(true);
  });

  it('is not the ordinary case: no error means nothing to explain', () => {
    expect(neverSucceeded(rowFixture())).toBe(false);
    expect(neverSucceeded(rowFixture({ refstateObservedAt: null, worktreeObservedAt: null }))).toBe(
      false,
    );
  });

  it('is not stale-but-once-known — either observation is enough to have been read', () => {
    expect(
      neverSucceeded(
        rowFixture({
          errorKind: 'STORE_OFFLINE',
          refstateObservedAt: 1_700_000_000,
          worktreeObservedAt: null,
        }),
      ),
    ).toBe(false);
    expect(
      neverSucceeded(
        rowFixture({
          errorKind: 'STORE_OFFLINE',
          refstateObservedAt: null,
          worktreeObservedAt: 1_700_000_000,
        }),
      ),
    ).toBe(false);
  });

  it('reads three columns and no others, so a caller may pass any row shape carrying them', () => {
    expect(
      neverSucceeded({
        errorKind: 'REPO_UNREADABLE',
        refstateObservedAt: null,
        worktreeObservedAt: null,
      }),
    ).toBe(true);
  });
});

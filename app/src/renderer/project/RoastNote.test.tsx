import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { ProjectPageDepsContext, type ProjectPageDeps } from './deps';
import { RoastNote, toRoastInput } from './RoastNote';
import { detailFixture, locationFixture, NOW, rowFixture } from './testFixtures';

afterEach(cleanup);

const deps: ProjectPageDeps = {
  request: (() => Promise.resolve({})) as unknown as ProjectPageDeps['request'],
  relocate: () => Promise.resolve({ kind: 'cancelled' }),
  openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
  subscribe: () => () => undefined,
  now: () => NOW,
};

function draw(props: Parameters<typeof RoastNote>[0]): void {
  render(
    <ProjectPageDepsContext.Provider value={deps}>
      <RoastNote {...props} />
    </ProjectPageDepsContext.Provider>,
  );
}

// The never-succeeded predicate is asserted where it is declared, in `errors/errorKind.test.ts`.
// This file asserts only that the adapter passes its answer through.
describe('the adapter', () => {
  it('phrases from the shown location and never from the aggregate', () => {
    const shown = locationFixture({ isPrimary: false, stashCount: 3, isDirty: false });
    const primary = locationFixture({ isPrimary: true, isDirty: true });
    const input = toRoastInput({
      detail: detailFixture({ locations: [primary, shown], row: rowFixture({ isDirty: true }) }),
      shown,
      primary,
      roastsEnabled: true,
      now: NOW,
    });
    expect(input?.shown.stashCount).toBe(3);
    expect(input?.shown.isDirty).toBe(false);
  });

  it('is null when there is no location to describe', () => {
    expect(
      toRoastInput({
        detail: detailFixture({ locations: [] }),
        shown: null,
        primary: null,
        roastsEnabled: true,
        now: NOW,
      }),
    ).toBeNull();
  });

  it('passes the never-succeeded answer through rather than deciding it here', () => {
    const shown = locationFixture({ stashCount: 3 });
    const unread = rowFixture({
      errorKind: 'PERMISSION_DENIED',
      refstateObservedAt: null,
      worktreeObservedAt: null,
    });
    const input = toRoastInput({
      detail: detailFixture({ locations: [shown], row: unread }),
      shown,
      primary: shown,
      roastsEnabled: true,
      now: NOW,
    });
    expect(input?.neverSucceeded).toBe(true);
  });

  it('narrows an interrupted operation §5.6 has no sentence for to nothing', () => {
    const shown = locationFixture({ interruptedOp: 'bisect' });
    const input = toRoastInput({
      detail: detailFixture({ locations: [shown] }),
      shown,
      primary: shown,
      roastsEnabled: true,
      now: NOW,
    });
    expect(input?.shown.interruptedOp).toBeNull();
  });
});

describe('the block', () => {
  it('renders §8.5.1 chrome around exactly the selected line', () => {
    const shown = locationFixture({ stashCount: 3 });
    draw({
      detail: detailFixture({ locations: [shown] }),
      shown,
      primary: shown,
      roastsEnabled: true,
    });
    expect(screen.getByTestId('cp-roast-label').textContent).toBe('NOTE');
    expect(screen.getByTestId('cp-roast-line').textContent).toBe(
      '3 stashes here. Nothing pushes a stash.',
    );
  });

  it('renders no block at all when nothing matches — not an empty block, not a placeholder', () => {
    const shown = locationFixture();
    draw({
      detail: detailFixture({ locations: [shown] }),
      shown,
      primary: shown,
      roastsEnabled: true,
    });
    expect(screen.queryByTestId('cp-roast')).toBeNull();
  });

  it('renders nothing when the switch is off', () => {
    const shown = locationFixture({ stashCount: 3 });
    draw({
      detail: detailFixture({ locations: [shown] }),
      shown,
      primary: shown,
      roastsEnabled: false,
    });
    expect(screen.queryByTestId('cp-roast')).toBeNull();
  });

  it('says nothing about a project the scan could never read', () => {
    const shown = locationFixture({ stashCount: 3 });
    const unread = rowFixture({
      errorKind: 'PERMISSION_DENIED',
      refstateObservedAt: null,
      worktreeObservedAt: null,
    });
    draw({
      detail: detailFixture({ locations: [shown], row: unread }),
      shown,
      primary: shown,
      roastsEnabled: true,
    });
    expect(screen.queryByTestId('cp-roast')).toBeNull();
  });

  it('never names itself and never carries an accent edge', () => {
    const shown = locationFixture({ stashCount: 3 });
    draw({
      detail: detailFixture({ locations: [shown] }),
      shown,
      primary: shown,
      roastsEnabled: true,
    });
    const block = screen.getByTestId('cp-roast');
    expect(block.textContent).not.toMatch(/roast/i);
    expect(block.className).toContain('cp-roast');
  });
});

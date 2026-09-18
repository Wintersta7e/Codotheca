import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectDetail, TargetId } from '../../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { detailFixture, locationFixture, NOW, rowFixture, targetFixture } from '../testFixtures';
import { CHECKING_NOTE, UNINSTALL_LABEL, UNINSTALL_OPEN_LABEL } from '../uninstall/uninstallCopy';
import { NO_APP_STATEMENT, Rail, terminalTarget, type RailProps } from './Rail';

afterEach(cleanup);

function draw(
  detail: ProjectDetail,
  request: ReturnType<typeof vi.fn> = vi.fn(() => Promise.resolve(1)),
): { request: ReturnType<typeof vi.fn> } {
  const deps: ProjectPageDeps = {
    request: request as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
  render(
    <ProjectPageDepsContext.Provider value={deps}>
      <Rail detail={detail} shown={detail.locations[0] ?? null} onChanged={vi.fn()} />
    </ProjectPageDepsContext.Provider>,
  );
  return { request };
}

describe('the primary control', () => {
  it('launches the copy the page is showing, with the resolved target', async () => {
    const detail = detailFixture();
    const { request } = draw(detail);
    fireEvent.click(screen.getByTestId('cp-cta'));
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith('projects.launch', {
        projectId: detail.row.id,
        locationId: detail.locations[0]?.location.id,
        targetId: detail.resolvedTarget?.target.id,
      }),
    );
  });

  it('issues exactly one launch however fast the button is pressed twice', async () => {
    const { request } = draw(detailFixture());
    const cta = screen.getByTestId('cp-cta');
    fireEvent.click(cta);
    fireEvent.click(cta);
    await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
  });

  it('is a statement, not a disabled button, when no copy is reachable', () => {
    draw(detailFixture({ locations: [locationFixture({ presence: 'missing' })] }));
    expect(screen.queryByTestId('cp-cta')).toBeNull();
    expect(screen.getByTestId('cp-cta-statement').textContent).toBe('NO REACHABLE COPY');
  });

  it('sends the user to OPENS IN when nothing resolves, rather than nowhere', () => {
    draw(detailFixture({ resolvedTarget: null }));
    expect(screen.getByTestId('cp-cta').textContent).toBe('CHOOSE AN APP');
    fireEvent.click(screen.getByTestId('cp-cta'));
    expect(document.activeElement).toBe(screen.getByTestId('cp-opensin'));
  });

  it('states the absence instead when there is no editor to choose', () => {
    draw(
      detailFixture({
        resolvedTarget: null,
        targets: [targetFixture({ id: 9 as unknown as TargetId, kind: 'terminal', name: 'Shell' })],
      }),
    );
    expect(screen.queryByTestId('cp-cta')).toBeNull();
    expect(screen.getByTestId('cp-cta-statement').textContent).toBe(NO_APP_STATEMENT);
  });
});

describe('TERMINAL', () => {
  it('picks the terminal target by kind and lowest sort index', () => {
    const a = targetFixture({ id: 4 as unknown as TargetId, kind: 'terminal', sortIndex: 2 });
    const b = targetFixture({ id: 5 as unknown as TargetId, kind: 'terminal', sortIndex: 0 });
    expect(terminalTarget([a, b])?.id).toBe(b.id);
    expect(terminalTarget([targetFixture({ kind: 'editor' })])).toBeNull();
  });

  it('is absent, not disabled, when no terminal is known', () => {
    draw(detailFixture({ targets: [targetFixture({ kind: 'editor' })] }));
    expect(screen.queryByTestId('cp-terminal')).toBeNull();
  });

  it('launches the shown copy in the terminal target, not in the resolved editor', async () => {
    const detail = detailFixture();
    const { request } = draw(detail);
    fireEvent.click(screen.getByTestId('cp-terminal'));
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith('projects.launch', {
        projectId: detail.row.id,
        locationId: detail.locations[0]?.location.id,
        targetId: terminalTarget(detail.targets)?.id,
      }),
    );
  });
});

describe('the stat blocks', () => {
  /**
   * `2.1h`, not `2h 07m`: R20's `formatPlaytime` is the product's one playtime formatter and it
   * renders hours with one decimal of precision. §7.8's bench scrim is the other ledger and the
   * other grammar, and folding them together is what "two ledgers, never merged" forbids.
   */
  it('prints launched-session time and the birth year side by side', () => {
    draw(detailFixture({ playtimeSeconds: 2 * 3600 + 7 * 60 }));
    expect(screen.getByTestId('cp-stat-playtime').textContent).toBe('2.1h');
    expect(screen.getByTestId('cp-stat-birth').textContent).toBe('2019');
  });

  it('prints the one zero this product may print, because that ledger starts at install', () => {
    draw(detailFixture({ playtimeSeconds: 0 }));
    expect(screen.getByTestId('cp-stat-playtime').textContent).toBe('0h');
  });

  it('never invents a birth year', () => {
    draw(detailFixture({ row: rowFixture({ birthYear: null }) }));
    expect(screen.getByTestId('cp-stat-birth').textContent).toBe('NOT KNOWN');
  });

  it('draws no total over the two ledgers — playtime is never added to anything', () => {
    draw(detailFixture({ playtimeSeconds: 3600 }));
    const rail = screen.getByTestId('cp-rail');
    expect(rail.textContent).not.toMatch(/TOTAL|COMBINED|OVERALL/i);
  });
});

/**
 * [p2] §24.8's affordance. The pre-flight fetches from the remote, so what the rail may do
 * before a press is the criterion — not what it looks like afterwards.
 */
describe("the rail's removal slot", () => {
  function drawWith(over: Partial<RailProps>): { onOpen: ReturnType<typeof vi.fn> } {
    const detail = detailFixture();
    const onOpen = vi.fn();
    const deps: ProjectPageDeps = {
      request: (() => Promise.resolve(1)) as unknown as ProjectPageDeps['request'],
      relocate: () => Promise.resolve({ kind: 'cancelled' }),
      uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
      openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
      subscribe: () => () => undefined,
      now: () => NOW,
    };
    render(
      <ProjectPageDepsContext.Provider value={deps}>
        <Rail
          detail={detail}
          shown={detail.locations[0] ?? null}
          onChanged={vi.fn()}
          onOpenUninstall={onOpen}
          {...over}
        />
      </ProjectPageDepsContext.Provider>,
    );
    return { onOpen };
  }

  it('offers nothing at all when the page does not hand it an opener', () => {
    draw(detailFixture());
    expect(screen.queryByTestId('cp-uninstall-open')).toBeNull();
    expect(screen.queryByTestId('cp-uninstall')).toBeNull();
  });

  it('draws the opener, and no verdict, before anything has been pressed', () => {
    const { onOpen } = drawWith({});
    const opener = screen.getByTestId('cp-uninstall-open');
    expect(opener.textContent).toBe(UNINSTALL_OPEN_LABEL);
    // Rendering alone must start nothing: the pre-flight is a network fetch.
    expect(onOpen).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: UNINSTALL_LABEL })).toBeNull();
  });

  it('asks for the verdict only on the press', () => {
    const { onOpen } = drawWith({});
    const opener = screen.getByTestId('cp-uninstall-open');
    fireEvent.mouseOver(opener);
    fireEvent.focus(opener);
    expect(onOpen).not.toHaveBeenCalled();
    fireEvent.click(opener);
    expect(onOpen).toHaveBeenCalledTimes(1);
  });

  it('shows checking in place of the opener while the pre-flight is in flight', () => {
    drawWith({ uninstallVerdict: null });
    expect(screen.queryByTestId('cp-uninstall-open')).toBeNull();
    expect(screen.getByTestId('cp-rail').textContent).toContain(CHECKING_NOTE);
  });

  it('has no slot on a project whose only copy is gone', () => {
    const absent = detailFixture({ locations: [locationFixture({ presence: 'missing' })] });
    const onOpen = vi.fn();
    const deps: ProjectPageDeps = {
      request: (() => Promise.resolve(1)) as unknown as ProjectPageDeps['request'],
      relocate: () => Promise.resolve({ kind: 'cancelled' }),
      uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
      openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
      subscribe: () => () => undefined,
      now: () => NOW,
    };
    render(
      <ProjectPageDepsContext.Provider value={deps}>
        <Rail
          detail={absent}
          shown={absent.locations[0] ?? null}
          onChanged={vi.fn()}
          onOpenUninstall={onOpen}
        />
      </ProjectPageDepsContext.Provider>,
    );
    expect(screen.queryByTestId('cp-uninstall-open')).toBeNull();
  });
});

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectDetail, RootId, TargetId } from '../../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { detailFixture, locationFixture, NOW, targetFixture } from '../testFixtures';
import { fileManagerTarget, LocationsPanel } from './LocationsPanel';
import { required } from '../../../shared/required';

afterEach(cleanup);

const ROOT = 3 as RootId;

interface Drawn {
  readonly request: ReturnType<typeof vi.fn>;
  readonly relocate: ReturnType<typeof vi.fn>;
  readonly onShow: ReturnType<typeof vi.fn>;
  readonly onChanged: ReturnType<typeof vi.fn>;
  readonly view: ReturnType<typeof render>;
}

function draw(detail: ProjectDetail, over: Partial<ProjectPageDeps> = {}): Drawn {
  const request = vi.fn((name: string) => {
    if (name === 'targets.list') {
      return Promise.resolve({ rows: detail.targets, resolved: detail.resolvedTarget });
    }
    return Promise.resolve({});
  });
  const relocate = vi.fn(() => Promise.resolve({ kind: 'cancelled' as const }));
  const onShow = vi.fn();
  const onChanged = vi.fn();
  const deps: ProjectPageDeps = {
    request: request as unknown as ProjectPageDeps['request'],
    relocate,
    uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
    installStart: () =>
      Promise.resolve({ kind: 'started' as const, start: { runId: 1, refusedBecause: null } }),
    installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
    pickRoot: () => Promise.resolve({ kind: 'cancelled' as const }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' as const }),
    subscribe: () => () => undefined,
    now: () => NOW,
    ...over,
  };
  const view = render(
    <ProjectPageDepsContext.Provider value={deps}>
      <LocationsPanel detail={detail} shownId={null} onShow={onShow} onChanged={onChanged} />
    </ProjectPageDepsContext.Provider>,
  );
  return { request, relocate, onShow, onChanged, view };
}

const twoCopies = (): ProjectDetail =>
  detailFixture({
    locations: [
      locationFixture({ isPrimary: true }),
      locationFixture({ isPrimary: false, presence: 'offline', branch: 'main' }),
    ],
    associationKind: 'strong',
  });

describe('the rows', () => {
  it('plates exactly one row PRIMARY and tags every other COPY', () => {
    draw(twoCopies());
    expect(screen.getAllByTestId('cp-loc-tag').map((n) => n.textContent)).toEqual([
      'PRIMARY',
      'COPY',
    ]);
  });

  it('draws all four presence states, not the design’s one', () => {
    draw(
      detailFixture({
        locations: [
          locationFixture({ isPrimary: true }),
          locationFixture({ presence: 'offline', isPrimary: false }),
          locationFixture({ presence: 'missing', isPrimary: false }),
          locationFixture({ presence: 'unscanned', isPrimary: false, coveringRootId: ROOT }),
        ],
      }),
    );
    expect(screen.getAllByTestId('cp-loc-state').map((n) => n.textContent)).toEqual([
      'SAME COMMIT',
      'OFFLINE',
      'MISSING',
      'NOT SCANNED',
    ]);
  });

  it('adopts a present copy through a real button carrying aria-pressed, and draws no marker', () => {
    const { onShow } = draw(twoCopies());
    const adopt = screen.getAllByTestId('cp-loc-adopt')[0];
    expect(adopt?.tagName).toBe('BUTTON');
    expect(adopt?.getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(required(adopt, 'ADOPT button'));
    expect(onShow).toHaveBeenCalled();
    expect(screen.queryByText('CURRENT')).toBeNull();
  });

  it('never puts a nested button inside the adopt button', () => {
    const { view } = draw(twoCopies());
    for (const b of view.container.querySelectorAll('button')) {
      expect(b.querySelector('button')).toBeNull();
    }
  });

  it('paints the primary row in this project’s own jewel, not in a second derivation', () => {
    const { view } = draw(twoCopies());
    const panel = required(view.container.querySelector<HTMLElement>('.cp-loc'), 'locations panel');
    expect(panel.style.getPropertyValue('--cdt-jewel')).toMatch(/^oklch\(/);
    expect(panel.style.getPropertyValue('--cdt-jewel-55')).toContain('/ .55)');
  });
});

describe('the header note and the footer', () => {
  it('names the exception in the header', () => {
    draw(twoCopies());
    expect(screen.getByTestId('cp-loc-header').textContent).toBe('2 COPIES · ONE OFFLINE');
  });

  it('draws the association evidence, and no footer on a single copy', () => {
    draw(twoCopies());
    expect(screen.getByTestId('cp-loc-footer').textContent).toBe('SAME LINEAGE · SAME REMOTE');
    cleanup();
    draw(detailFixture());
    expect(screen.queryByTestId('cp-loc-footer')).toBeNull();
  });

  it('renders neither the superseded footer nor the token FORGET anywhere', () => {
    const { view } = draw(twoCopies());
    const html = view.container.innerHTML;
    expect(html).not.toContain('SAME ROOT COMMIT');
    expect(html).not.toMatch(/FORGET/i);
    expect(html).not.toMatch(/DIVERGED|BEHIND PRIMARY/);
  });
});

describe('the actions', () => {
  it('re-resolves the target for that copy before launching it', async () => {
    const detail = twoCopies();
    const { request } = draw(detail);
    fireEvent.click(
      required(screen.getAllByRole('button', { name: 'OPEN' })[0], 'first OPEN button'),
    );
    await waitFor(() => {
      expect(request).toHaveBeenCalledWith('targets.list', {
        projectId: detail.row.id,
        locationId: detail.locations[0]?.location.id,
      });
    });
    await waitFor(() => {
      expect(request).toHaveBeenCalledWith('projects.launch', {
        projectId: detail.row.id,
        locationId: detail.locations[0]?.location.id,
        targetId: detail.resolvedTarget?.target.id,
      });
    });
  });

  it('picks the file manager by kind for REVEAL, and hides it when none is known', () => {
    const fm = targetFixture({ id: 8 as TargetId, kind: 'file_manager' });
    expect(fileManagerTarget([targetFixture({ kind: 'editor' }), fm])?.id).toBe(fm.id);
    draw(twoCopies());
    expect(screen.queryByRole('button', { name: 'REVEAL' })).toBeNull();
  });

  it('offers an unreachable copy RELOCATE and nothing else, and sends only the location id', async () => {
    const detail = twoCopies();
    const relocate = vi.fn(() => Promise.resolve({ kind: 'relocated' as const, location: {} }));
    const { onChanged } = draw(detail, {
      relocate: relocate as unknown as ProjectPageDeps['relocate'],
    });
    const row = required(screen.getAllByTestId('cp-loc-row')[1], 'second location row');
    expect(row.textContent).not.toContain('OPEN');
    fireEvent.click(screen.getByRole('button', { name: 'RELOCATE' }));
    await waitFor(() => {
      expect(onChanged).toHaveBeenCalled();
    });
    // The renderer originates no path: the id is the whole of what crosses the channel.
    expect(relocate).toHaveBeenCalledWith(detail.locations[1]?.location.id);
    expect(relocate.mock.calls[0]).toHaveLength(1);
  });

  it('enables the covering root by id, with no path on the wire', async () => {
    const detail = detailFixture({
      locations: [
        locationFixture({ isPrimary: true }),
        locationFixture({ isPrimary: false, presence: 'unscanned', coveringRootId: ROOT }),
      ],
    });
    const { request } = draw(detail);
    fireEvent.click(screen.getByRole('button', { name: 'ENABLE ROOT' }));
    await waitFor(() => {
      expect(request).toHaveBeenCalledWith('roots.setEnabled', { id: 3, enabled: true });
    });
    const [, args] = (request.mock.calls.find((c) => c[0] === 'roots.setEnabled') ?? []) as [
      string,
      Record<string, unknown>,
    ];
    expect(Object.keys(args).sort()).toEqual(['enabled', 'id']);
  });
});

describe('the notes', () => {
  it('carries the offline note verbatim on an offline row', () => {
    draw(twoCopies());
    expect(screen.getByTestId('cp-loc-note').textContent).toBe(
      'The drive is not mounted. This copy is frozen, not rotting — its condition stops here rather than decaying.',
    );
  });

  it('carries the different-commit note and no directional claim', () => {
    draw(
      detailFixture({
        locations: [
          locationFixture({ isPrimary: true }),
          locationFixture({ isPrimary: false, headComparison: 'different_commit' }),
        ],
      }),
    );
    expect(screen.getByTestId('cp-loc-note').textContent).toBe(
      'This copy is on a different commit. Opening it is not the same as opening the project.',
    );
  });

  it('renders nothing at all for a project with no locations', () => {
    const { view } = draw(detailFixture({ locations: [] }));
    expect(view.container.firstChild).toBeNull();
  });
});

describe('contrast', () => {
  it('uses no class this panel is barred from — --text-4 and --text-5 appear nowhere in it', () => {
    const { view } = draw(twoCopies());
    expect(view.container.innerHTML).not.toMatch(/text-4|text-5/);
  });
});

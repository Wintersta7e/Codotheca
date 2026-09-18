import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectDetail, ProjectId, UninstallVerdict } from '../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from './deps';
import { primaryLocation, ProjectPageView, shownLocation } from './ProjectPage';
import { detailFixture, locationFixture, NOW, rowFixture } from './testFixtures';
import { blockerSentence, UNINSTALL_LABEL } from './uninstall/uninstallCopy';

afterEach(cleanup);

function depsFor(detail: ProjectDetail): ProjectPageDeps {
  return {
    request: ((name: string) => {
      if (name === 'projects.get') return Promise.resolve(detail);
      if (name === 'art.url') return Promise.resolve('codotheca://art/aa/hero');
      return Promise.resolve({});
    }) as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
}

function mount(detail: ProjectDetail, onBack = vi.fn()): { onBack: typeof onBack } {
  render(
    <ProjectPageDepsContext.Provider value={depsFor(detail)}>
      <ProjectPageView
        projectId={7 as unknown as ProjectId}
        onBack={onBack}
        onOpenProject={vi.fn()}
      />
    </ProjectPageDepsContext.Provider>,
  );
  return { onBack };
}

/** `resolveKey` matches on `code`, so a bare `key` would be a keystroke the product never sees. */
function press(element: Element, code: string, key = code, init: KeyboardEventInit = {}): void {
  element.dispatchEvent(new KeyboardEvent('keydown', { key, code, bubbles: true, ...init }));
}

describe('the shell', () => {
  it('draws exactly two tabs and no disabled third', async () => {
    mount(detailFixture());
    await screen.findByRole('tab', { name: 'OVERVIEW' });
    const tabs = screen.getAllByRole('tab');
    expect(tabs.map((t) => t.textContent)).toEqual(['OVERVIEW', 'ACTIVITY']);
    for (const tab of tabs) expect(tab.hasAttribute('disabled')).toBe(false);
  });

  it('shows the resolved path_display and never an operational path', async () => {
    mount(detailFixture());
    const path = await screen.findByTestId('cp-bar-path');
    expect(path.textContent).toBe('~/work/aurora');
  });

  it('moves between tabs on the arrow keys and back on Escape', async () => {
    const { onBack } = mount(detailFixture());
    const root = await screen.findByTestId('cp-page');
    press(root, 'ArrowRight');
    await waitFor(() => {
      expect(screen.getByRole('tab', { name: 'ACTIVITY' }).getAttribute('aria-selected')).toBe(
        'true',
      );
    });
    press(root, 'ArrowLeft');
    await waitFor(() => {
      expect(screen.getByRole('tab', { name: 'OVERVIEW' }).getAttribute('aria-selected')).toBe(
        'true',
      );
    });
    press(root, 'Escape');
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it('leaves the arrow keys to the caret while a field has focus', async () => {
    mount(detailFixture());
    const root = await screen.findByTestId('cp-page');
    const field = document.createElement('textarea');
    root.appendChild(field);
    field.focus();
    press(field, 'ArrowRight');
    expect(screen.getByRole('tab', { name: 'OVERVIEW' }).getAttribute('aria-selected')).toBe(
      'true',
    );
  });

  it('binds no shelf key — no Space, no Enter, no P', async () => {
    const { onBack } = mount(detailFixture());
    const root = await screen.findByTestId('cp-page');
    for (const code of ['Space', 'Enter', 'KeyP', 'ArrowUp', 'ArrowDown']) {
      press(root, code);
    }
    expect(onBack).not.toHaveBeenCalled();
    expect(screen.getByRole('tab', { name: 'OVERVIEW' }).getAttribute('aria-selected')).toBe(
      'true',
    );
  });

  it('leaves the quick switch to whoever owns the palette', async () => {
    const { onBack } = mount(detailFixture());
    const root = await screen.findByTestId('cp-page');
    press(root, 'Space', ' ', { altKey: true });
    expect(onBack).not.toHaveBeenCalled();
    expect(screen.getByRole('tab', { name: 'OVERVIEW' }).getAttribute('aria-selected')).toBe(
      'true',
    );
  });

  it('pairs each tab with the panel it controls', async () => {
    mount(detailFixture());
    const panel = await screen.findByTestId('cp-tabpanel');
    const selected = screen.getByRole('tab', { name: 'OVERVIEW' });
    expect(selected.getAttribute('aria-controls')).toBe(panel.id);
    expect(panel.getAttribute('aria-labelledby')).toBe(selected.id);
  });

  it('says the project could not be read rather than drawing an empty page', async () => {
    const failing: ProjectPageDeps = {
      request: () => Promise.reject(new Error('nope')),
      relocate: () => Promise.resolve({ kind: 'cancelled' }),
      uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
      openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
      subscribe: () => () => undefined,
      now: () => NOW,
    };
    render(
      <ProjectPageDepsContext.Provider value={failing}>
        <ProjectPageView
          projectId={7 as unknown as ProjectId}
          onBack={vi.fn()}
          onOpenProject={vi.fn()}
        />
      </ProjectPageDepsContext.Provider>,
    );
    expect(await screen.findByRole('button', { name: 'TRY AGAIN' })).toBeTruthy();
    expect(screen.queryByTestId('cp-tabpanel')).toBeNull();
  });
});

/**
 * §8.3 filters `is:pinned` client-side, so the renderer flips the bit in its own projection the
 * moment the control is pressed and `projects/flags_changed` reconciles. A pin that waited for the
 * round trip would leave the query and the mark disagreeing about the same project.
 */
describe('pinning from the hero', () => {
  function mountWith(request: ReturnType<typeof vi.fn>): void {
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
        <ProjectPageView
          projectId={7 as unknown as ProjectId}
          onBack={vi.fn()}
          onOpenProject={vi.fn()}
        />
      </ProjectPageDepsContext.Provider>,
    );
  }

  const answering = (
    detail: ProjectDetail,
    setFlags: () => Promise<unknown>,
  ): ReturnType<typeof vi.fn> =>
    vi.fn((name: string) => {
      if (name === 'projects.get') return Promise.resolve(detail);
      if (name === 'projects.setFlags') return setFlags();
      if (name === 'art.url') return Promise.resolve('codotheca://art/aa/hero');
      return Promise.resolve({});
    });

  it('flips the mark without waiting for the answer', async () => {
    const detail = detailFixture({ row: rowFixture({ isPinned: false }) });
    // A command that never resolves: whatever the mark does next, it did not wait for this.
    const request = answering(detail, () => new Promise<never>(() => undefined));
    mountWith(request);
    fireEvent.click(await screen.findByRole('button', { name: 'Pin aurora' }));
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Unpin aurora' })).toBeTruthy();
    });
  });

  it('touches only the pinned flag — archived and hidden are other controls', async () => {
    const detail = detailFixture({ row: rowFixture({ isPinned: false }) });
    const request = answering(detail, () => Promise.resolve({}));
    mountWith(request);
    fireEvent.click(await screen.findByRole('button', { name: 'Pin aurora' }));
    await waitFor(() => {
      expect(request.mock.calls.some((c) => c[0] === 'projects.setFlags')).toBe(true);
    });
    const call = request.mock.calls.find((c) => c[0] === 'projects.setFlags');
    expect(call?.[1]).toEqual({ id: 7, isPinned: true, isArchived: null, isHidden: null });
  });

  it('goes back to the core’s answer when the write is refused', async () => {
    const detail = detailFixture({ row: rowFixture({ isPinned: false }) });
    const request = answering(detail, () => Promise.reject(new Error('refused')));
    mountWith(request);
    fireEvent.click(await screen.findByRole('button', { name: 'Pin aurora' }));
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Pin aurora' })).toBeTruthy();
    });
  });

  it('unpins a pinned project rather than pinning it again', async () => {
    const detail = detailFixture({ row: rowFixture({ isPinned: true }) });
    const request = answering(detail, () => Promise.resolve({}));
    mountWith(request);
    fireEvent.click(await screen.findByRole('button', { name: 'Unpin aurora' }));
    await waitFor(() => {
      expect(request.mock.calls.some((c) => c[0] === 'projects.setFlags')).toBe(true);
    });
    const call = request.mock.calls.find((c) => c[0] === 'projects.setFlags');
    expect(call?.[1]).toEqual({ id: 7, isPinned: false, isArchived: null, isHidden: null });
  });
});

describe('the mounted body', () => {
  it('draws the rail beside the hero and the Overview panels beside them', async () => {
    mount(detailFixture());
    await screen.findByTestId('cp-rail');
    expect(screen.getByTestId('cp-loc-header')).toBeTruthy();
    expect(screen.getByTestId('cp-readme-header')).toBeTruthy();
    expect(screen.getByTestId('cp-note-label')).toBeTruthy();
  });

  it('mounts one tab at a time — the ACTIVITY chart does not exist while OVERVIEW is shown', async () => {
    mount(detailFixture());
    await screen.findByTestId('cp-loc-header');
    expect(screen.queryByTestId('cp-act-note')).toBeNull();
    press(await screen.findByTestId('cp-page'), 'ArrowRight');
    await waitFor(() => {
      expect(screen.getByTestId('cp-act-note')).toBeTruthy();
    });
    expect(screen.queryByTestId('cp-loc-header')).toBeNull();
    expect(screen.queryByTestId('cp-note-label')).toBeNull();
  });

  it('adopts a copy from the Locations panel, and the page follows that one', async () => {
    const detail = detailFixture({
      locations: [locationFixture({ isPrimary: true }), locationFixture({ isPrimary: false })],
    });
    mount(detail);
    const adopts = await screen.findAllByTestId('cp-loc-adopt');
    expect(adopts).toHaveLength(2);
    fireEvent.click(adopts[1] as HTMLElement);
    const second = String(detail.locations[1]?.location.id);
    await waitFor(() => {
      expect(screen.getByTestId('cp-tabpanel').dataset['shownLocation']).toBe(second);
    });
    expect(screen.getAllByTestId('cp-loc-adopt')[1]?.getAttribute('aria-pressed')).toBe('true');
  });

  /**
   * R45's text-entry guard lives in the key table and this is the surface that proves it: `Esc`
   * typed into the note reverts the edit and stays in the field rather than leaving the page.
   */
  it('leaves Escape to the note field rather than backing out of the page', async () => {
    const { onBack } = mount(detailFixture());
    fireEvent.click(await screen.findByTestId('cp-note-empty'));
    const field = screen.getByTestId('cp-note-field');
    fireEvent.change(field, { target: { value: 'discarded' } });
    // `fireEvent`, not the raw dispatch `press` uses: the assertion is about what React does
    // with the event, so it has to go through React's own delegation and be act-wrapped.
    fireEvent.keyDown(field, { key: 'Escape', code: 'Escape' });
    expect(onBack).not.toHaveBeenCalled();
    expect(screen.getByTestId('cp-note-empty')).toBeTruthy();
  });
});

describe('the shown location', () => {
  it('defaults to the primary', () => {
    const detail = detailFixture();
    expect(shownLocation(detail, null)?.location.id).toBe(primaryLocation(detail)?.location.id);
  });

  it('falls back to the primary when the held id is no longer in the set', () => {
    const detail = detailFixture();
    const gone = 999 as unknown as ProjectDetail['locations'][number]['location']['id'];
    expect(shownLocation(detail, gone)?.isPrimary).toBe(true);
  });

  it('is null when the project has no locations at all', () => {
    const detail = detailFixture({ locations: [] });
    expect(shownLocation(detail, null)).toBeNull();
    expect(primaryLocation(detail)).toBeNull();
  });

  it('takes the first row as primary when no row is flagged', () => {
    const detail = detailFixture({
      locations: [locationFixture({ isPrimary: false }), locationFixture({ isPrimary: false })],
    });
    expect(primaryLocation(detail)).toBe(detail.locations[0]);
  });
});

/**
 * [p2] §24.8's removal, mounted. The unit tests prove the hook's timing and the control's DOM;
 * this proves the page joins them — the defect that shipped once was two complete, tested
 * features that no surface mounted.
 */
describe('the removal the page offers', () => {
  function mountWith(
    verdict: UninstallVerdict,
    uninstall: ReturnType<typeof vi.fn>,
  ): { request: ReturnType<typeof vi.fn> } {
    const detail = detailFixture();
    const request = vi.fn((name: string) => {
      if (name === 'projects.get') return Promise.resolve(detail);
      if (name === 'art.url') return Promise.resolve('codotheca://art/aa/hero');
      if (name === 'locations.uninstallPreflight') return Promise.resolve(verdict);
      return Promise.resolve({});
    });
    const deps: ProjectPageDeps = {
      ...depsFor(detail),
      request: request as unknown as ProjectPageDeps['request'],
      uninstall: uninstall as unknown as ProjectPageDeps['uninstall'],
    };
    render(
      <ProjectPageDepsContext.Provider value={deps}>
        <ProjectPageView
          projectId={7 as unknown as ProjectId}
          onBack={vi.fn()}
          onOpenProject={vi.fn()}
        />
      </ProjectPageDepsContext.Provider>,
    );
    return { request };
  }

  const safe = (): UninstallVerdict =>
    ({
      disposition: 'safe',
      blockers: [],
      remoteVerifiedAt: null,
      trashAvailable: true,
      computedAt: NOW,
    }) as unknown as UninstallVerdict;

  const blocked = (): UninstallVerdict =>
    ({
      disposition: 'blocked',
      blockers: ['unpushed_commits'],
      remoteVerifiedAt: null,
      trashAvailable: true,
      computedAt: NOW,
    }) as unknown as UninstallVerdict;

  it('mounts the affordance and runs no pre-flight until it is pressed', async () => {
    const { request } = mountWith(safe(), vi.fn());
    await screen.findByTestId('cp-uninstall-open');
    const asked = request.mock.calls.map((call) => String((call as unknown[])[0]));
    expect(asked).not.toContain('locations.uninstallPreflight');
  });

  it('reaches the enabled removal only through a verdict the core gave it', async () => {
    const uninstall = vi.fn(() => Promise.resolve({ kind: 'uninstalled', location: {} }));
    mountWith(safe(), uninstall);

    fireEvent.click(await screen.findByTestId('cp-uninstall-open'));
    const go = await screen.findByTestId('cp-rail').then(() => screen.findByText(UNINSTALL_LABEL));
    fireEvent.click(go);

    await waitFor(() => {
      expect(uninstall).toHaveBeenCalledTimes(1);
    });
  });

  it('names the blockers and offers nothing that reaches the removal', async () => {
    const uninstall = vi.fn(() => Promise.resolve({ kind: 'uninstalled', location: {} }));
    mountWith(blocked(), uninstall);

    fireEvent.click(await screen.findByTestId('cp-uninstall-open'));
    await screen.findByText(blockerSentence('unpushed_commits'));

    // The enabled control is absent, not merely disabled: an element a stray keyboard path
    // could still activate is a path past a non-safe disposition.
    expect(screen.queryByText(UNINSTALL_LABEL)).toBeNull();
    for (const button of screen.getAllByRole('button')) fireEvent.click(button);
    expect(uninstall).not.toHaveBeenCalled();
  });
});

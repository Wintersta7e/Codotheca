import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectDetail, ProjectId } from '../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from './deps';
import { primaryLocation, ProjectPageView, shownLocation } from './ProjectPage';
import { detailFixture, locationFixture, NOW } from './testFixtures';

afterEach(cleanup);

function depsFor(detail: ProjectDetail): ProjectPageDeps {
  return {
    request: ((name: string) => {
      if (name === 'projects.get') return Promise.resolve(detail);
      if (name === 'art.url') return Promise.resolve('codotheca://art/aa/hero');
      return Promise.resolve({});
    }) as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
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

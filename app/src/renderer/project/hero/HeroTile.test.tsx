import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectRow, SceneHash } from '../../../generated/protocol';
import { statusChips } from '../../card/chips';
import { LADDER_RUNGS } from '../../theme/tokens';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { NOW, rowFixture } from '../testFixtures';
import { HeroTile, heroIdentityLine } from './HeroTile';

afterEach(cleanup);

const deps: ProjectPageDeps = {
  request: (() =>
    Promise.resolve('codotheca://art/aa/hero')) as unknown as ProjectPageDeps['request'],
  relocate: () => Promise.resolve({ kind: 'cancelled' }),
  subscribe: () => () => undefined,
  now: () => NOW,
};

function draw(row: ProjectRow = rowFixture(), firstRunCompletedAt: number | null = null): void {
  render(
    <ProjectPageDepsContext.Provider value={deps}>
      <HeroTile row={row} heroHash={'aa' as SceneHash} firstRunCompletedAt={firstRunCompletedAt} />
    </ProjectPageDepsContext.Provider>,
  );
}

function frame(): HTMLElement {
  const node = document.querySelector('.cdt-card');
  if (node === null) throw new Error('the hero did not mount a card');
  return node as HTMLElement;
}

describe('the tile it mounts', () => {
  it('draws §7.7ʼs hero band table and not the grid tileʼs', () => {
    draw();
    expect(frame().getAttribute('data-surface')).toBe('hero');
  });

  it('mounts the class names the motion tier clamp selects', () => {
    draw();
    // A hero drawn from a private stylesheet would carry none of these and no gate would say so:
    // every clamp in the tier sheet would quietly select nothing on this surface.
    // `.cdt-strip` is the grid tile's hover data strip and does not exist on this surface;
    // everything else the tier sheet selects on a card is here.
    for (const name of ['cdt-card', 'cdt-plate', 'cdt-dot', 'cdt-card-halo', 'cdt-scanline']) {
      expect(document.querySelector(`.${name}`), name).not.toBeNull();
    }
  });
});

describe('the rank, which is the only state phase 1 has', () => {
  it('names the absence in the accessibility tree rather than leaving the frame unnamed', () => {
    draw();
    expect(screen.getByText('Completion not computed')).toBeTruthy();
  });

  it('paints no rung of the completion ladder, in either spelling', () => {
    draw();
    const html = document.body.innerHTML;
    for (const rung of LADDER_RUNGS) {
      expect(html).not.toContain(rung);
      // jsdom re-serialises an inline hex as `rgb(...)`, so the hex alone is a vacuous guard.
      const probe = document.createElement('div');
      probe.style.borderColor = rung;
      expect(html).not.toContain(probe.style.borderColor);
    }
  });

  it('renders no EVALUABLE line in band 5 — one surface states an absence once', () => {
    draw();
    expect(document.body.textContent).not.toMatch(/EVALUABLE/);
    expect(document.body.textContent).not.toMatch(/\d+\s*\/\s*\d+/);
  });
});

describe('band 4', () => {
  it('carries the one status-chip strip, not a second one written for the hero', () => {
    const row = rowFixture({ isDirty: true, ahead: 3, acknowledgedAt: NOW });
    draw(row);
    const expected = statusChips(row, NOW, null);
    expect(expected.length).toBeGreaterThan(0);
    for (const chip of expected) {
      expect(screen.getByText(chip.text)).toBeTruthy();
      expect(screen.getByText(chip.accessibleName)).toBeTruthy();
    }
  });

  it('draws no chip from a fact that was never observed', () => {
    draw(rowFixture({ isDirty: null, ahead: null, behind: null, interruptedOp: null }));
    for (const text of ['UNCOMMITTED', 'UNPUSHED', 'INTERRUPTED']) {
      expect(screen.queryByText(text)).toBeNull();
    }
  });

  it('omits BEHIND entirely with no fetch recorded — never BEHIND 0', () => {
    draw(rowFixture({ behind: 11, fetchHeadAt: null }));
    expect(screen.queryByText(/BEHIND/)).toBeNull();
  });

  it('offers no shields — LICENSE, BUILD, RELEASE and the star are out of phase 1', () => {
    draw(rowFixture({ isDirty: true }));
    expect(document.body.textContent).not.toMatch(/LICENSE|BUILD|RELEASE|★|GPL/);
  });

  it('draws NEW only once a first-run boundary exists to measure it against', () => {
    const fresh = rowFixture({ createdAt: NOW - 60, acknowledgedAt: null });
    draw(fresh, null);
    expect(screen.queryByText('NEW')).toBeNull();
    cleanup();
    draw(fresh, NOW - 3600);
    expect(screen.getByText('NEW')).toBeTruthy();
  });
});

describe('band 5', () => {
  it('reads first-commit year and language, owner-prefixed only when there is one', () => {
    expect(heroIdentityLine(rowFixture())).toBe('2019 · RUST');
    expect(heroIdentityLine(rowFixture({ owner: 'someone-else' }))).toBe(
      'SOMEONE-ELSE · 2019 · RUST',
    );
    expect(heroIdentityLine(rowFixture({ birthYear: null, primaryLanguage: null }))).toBeNull();
  });

  it('renders the line, and nothing at all when there is none', () => {
    draw();
    expect(screen.getByTestId('cp-hero-identity').textContent).toBe('2019 · RUST');
    cleanup();
    draw(rowFixture({ birthYear: null, primaryLanguage: null, owner: null }));
    expect(screen.queryByTestId('cp-hero-identity')).toBeNull();
  });
});

describe('what the hero does not do', () => {
  it('carries no material layer — dust, cobwebs, rust, cracks and overgrowth are all out', () => {
    draw(rowFixture({ conditionSignal: 'abandoned', lastCommitAt: NOW - 3000 * 86_400 }));
    // Class names, not bare words: `rust` is also a language and `2019 · RUST` is band 5 doing
    // its job. A word match here would fail against a correct hero and be "fixed" by deleting
    // the identity line.
    const html = document.body.innerHTML.toLowerCase();
    for (const layer of ['cobweb', 'dust', 'rust', 'crack', 'overgrowth', 'leaf', 'decay']) {
      expect(html, layer).not.toContain(`cdt-${layer}`);
      expect(html, layer).not.toContain(`cp-${layer}`);
    }
  });

  it('offers no pin control: the page carries no second copy of that toggle', () => {
    draw(rowFixture({ isPinned: true }));
    expect(screen.queryByRole('button', { name: /pin/i })).toBeNull();
  });

  it('asks for the hero rendition, never the card one', () => {
    const request = vi.fn(() => Promise.resolve('codotheca://art/aa/hero'));
    render(
      <ProjectPageDepsContext.Provider
        value={{ ...deps, request: request as unknown as ProjectPageDeps['request'] }}
      >
        <HeroTile row={rowFixture()} heroHash={'aa' as SceneHash} firstRunCompletedAt={null} />
      </ProjectPageDepsContext.Provider>,
    );
    expect(request).toHaveBeenCalledWith('art.url', { hash: 'aa', rendition: 'hero' });
  });
});

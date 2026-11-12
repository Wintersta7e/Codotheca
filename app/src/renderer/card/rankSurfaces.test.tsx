import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectRow } from '../../generated/protocol';
import { ListView } from '../shelf/ListView';
import { toShelfRow } from '../shelf/row';
import { withProjectDeps } from '../testing/deps';
import { makeProjectRow } from '../testing/projectRow';
import { noop } from '../noop';
import { HeroFrame } from './HeroFrame';
import { ProjectCard } from './ProjectCard';

afterEach(cleanup);

/** 9 of 10, `idle`, nothing else observed: an A with no adjustment. */
const computed = (over: Partial<ProjectRow> = {}): ProjectRow =>
  makeProjectRow({
    completionLit: 9,
    completionApplicable: 10,
    conditionSignal: 'idle',
    ...over,
  });

const card = (row: ProjectRow): HTMLElement =>
  render(
    <ProjectCard
      row={row}
      density={186}
      rendition="card"
      selected={false}
      focused={false}
      now={10_000}
      firstRunCompletedAt={null}
      session={null}
      haloOpacity={1}
      onActivate={vi.fn()}
      onOpen={vi.fn()}
      onTogglePin={vi.fn()}
      onStopSession={vi.fn()}
    />,
    { wrapper: withProjectDeps() },
  ).container;

const hero = (row: ProjectRow): HTMLElement =>
  render(
    <HeroFrame row={row} heroSrc="" halo={{ shadow: null, opacity: 1 }} chips={[]} pin={null}>
      <span />
    </HeroFrame>,
  ).container;

const list = (row: ProjectRow): HTMLElement =>
  render(
    <ListView
      rows={[toShelfRow(row)]}
      now={10_000}
      firstRunCompletedAt={0}
      selectedId={null}
      peek={null}
      onActivate={noop}
      onOpen={noop}
    />,
  ).container;

describe('RK1 — a computed project renders its rank glyph on all three surfaces', () => {
  it.each([
    ['grid card', card],
    ['hero', hero],
  ] as const)('the %s draws the glyph, its label and its name in band 3', (_name, draw) => {
    const container = draw(computed());
    const band = container.querySelector('.cdt-rank');
    expect(band?.getAttribute('data-rank')).toBe('A');
    const glyph = band?.querySelector<HTMLElement>('.cdt-rank-glyph');
    expect(glyph?.textContent).toBe('A');
    expect(glyph?.style.color).toBe('var(--rank-a)');
    expect(glyph?.style.textShadow).toBe('0 0 24px rgb(178 138 236 / .40)');
    const label = band?.querySelector<HTMLElement>('.cdt-rank-label');
    expect(label?.textContent).toBe('A RANK');
    expect(label?.style.color).toBe('var(--rank-a)');
    expect(band?.querySelector('.cdt-visually-hidden')?.textContent).toBe('Rank A');
    // The gap says there is no measurement; a computed rank never draws it.
    expect(container.querySelector('.cdt-frame-gap')).toBeNull();
    // The glyph is DOM text: no em dash and no uncomputed label stands beside it.
    expect(container.textContent).not.toContain('NOT COMPUTED');
  });

  it('the list fills its RANK column with the glyph in the rank ink', () => {
    const glyph = list(computed()).querySelector<HTMLElement>('.cdt-list-c2 .cdt-list-rank');
    expect(glyph?.textContent).toBe('A');
    expect(glyph?.getAttribute('data-rank')).toBe('A');
    expect(glyph?.style.color).toBe('var(--rank-a)');
  });

  it('reads the stored CI check on every surface, so a failing build is one rank lower', () => {
    const red = computed({ ciGreen: 'fail' });
    expect(card(red).querySelector('.cdt-rank')?.getAttribute('data-rank')).toBe('B');
    cleanup();
    expect(hero(red).querySelector('.cdt-rank')?.getAttribute('data-rank')).toBe('B');
    cleanup();
    expect(list(red).querySelector('.cdt-list-rank')?.textContent).toBe('B');
  });

  it('keeps the uncomputed treatment, and no glyph, while completion is uncomputed', () => {
    const uncomputed = makeProjectRow({ conditionSignal: 'abandoned', ciGreen: 'fail' });
    const tile = card(uncomputed);
    expect(tile.querySelector('.cdt-rank')?.hasAttribute('data-rank')).toBe(false);
    expect(tile.querySelector('.cdt-rank-glyph')?.textContent).toBe('—');
    expect(tile.querySelector('.cdt-rank-label')?.textContent).toBe('NOT COMPUTED');
    expect(tile.querySelector('.cdt-frame-gap')).not.toBeNull();
    cleanup();
    expect(list(uncomputed).querySelector('.cdt-list-rank')).toBeNull();
  });
});

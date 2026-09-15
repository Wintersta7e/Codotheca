import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectId, ProjectRow } from '../../generated/protocol';
import { ProjectCard, type ProjectCardProps } from '../card/ProjectCard';
import { flickerEligible } from '../motion/flicker';
import { Identity } from '../project/Identity';
import { QuickSwitch, type QuickSwitchProps } from '../palette/QuickSwitch';
import cardCss from '../styles/card.css?raw';
import motionCss from '../styles/motion.css?raw';
import { notClonedRow } from '../testing/projectRow';
import { withProjectDeps } from '../testing/deps';
import { ListView } from './ListView';
import { toShelfRow } from './row';

afterEach(() => {
  cleanup();
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
  document.body.innerHTML = '';
});

const NOW = 1_800_000_000;

/**
 * §23's render contract on the surfaces that draw it. AC-P2-23-1 says the row *renders*, and the
 * only way to assert that is to render it.
 *
 * Every assertion here is about an **absence**, and every absence is a refusal to claim
 * something: no dot, because nothing was measured; no age slot, because a not-cloned project has
 * no interaction clock; no `0`, because unknown is not zero; no playtime, because Peek's
 * carve-out for playtime's honest zero is about a *cloned* project never launched.
 */
const row = (over: Partial<ProjectRow> = {}): ProjectRow => notClonedRow(over);

const cardProps = (over: Partial<ProjectCardProps> = {}): ProjectCardProps => ({
  row: row(),
  density: 240,
  rendition: 'card',
  selected: false,
  focused: false,
  now: NOW,
  firstRunCompletedAt: null,
  session: null,
  haloOpacity: 1,
  onActivate: vi.fn(),
  onOpen: vi.fn(),
  onTogglePin: vi.fn(),
  onStopSession: vi.fn(),
  ...over,
});

const drawCard = (over: Partial<ProjectCardProps> = {}): HTMLElement =>
  render(<ProjectCard {...cardProps(over)} />, { wrapper: withProjectDeps() }).container;

const paletteProps = (rows: readonly ProjectRow[]): QuickSwitchProps => ({
  rows,
  query: '',
  cursor: 0,
  liveSessionProjectIds: new Set<ProjectId>(),
  nowSecs: NOW,
  effectsTier: 'full',
  jewelFor: () => '#4a9dff',
  onQueryChange: vi.fn(),
  onPoint: vi.fn(),
  onLaunch: vi.fn(),
  onOpenPage: vi.fn(),
  onClose: vi.fn(),
  onKeyDown: vi.fn(),
});

describe('§5.4a: no dot is drawn on any of the five surfaces', () => {
  it('draws none on the grid tile and none on the hero', () => {
    for (const surface of ['card', 'hero'] as const) {
      const container =
        surface === 'card'
          ? drawCard()
          : drawCard({ density: 186, row: row({ conditionSignal: null }) });
      expect(container.querySelector('[data-testid="cdt-dot"]')).toBeNull();
      cleanup();
    }
  });

  it('draws none on the list row', () => {
    const { container } = render(
      <ListView
        rows={[toShelfRow(row())]}
        now={NOW}
        firstRunCompletedAt={null}
        selectedId={null}
        peek={null}
        onActivate={vi.fn()}
        onOpen={vi.fn()}
      />,
    );
    expect(container.querySelector('.cdt-list-dot')).toBeNull();
    expect(container.textContent).not.toContain('Condition:');
  });

  it('draws none on the project page identity line', () => {
    const { container } = render(<Identity row={row()} />);
    expect(container.querySelector('[data-testid="cp-identity-dot"]')).toBeNull();
  });

  it('draws none in quick switch', () => {
    const { container } = render(<QuickSwitch {...paletteProps([row()])} />);
    expect(container.querySelector('.qs-dot')).toBeNull();
  });

  it('bites: a measured signal puts the dot back on the tile', () => {
    const container = drawCard({ row: row({ conditionSignal: 'idle' }) });
    expect(container.querySelector('[data-testid="cdt-dot"]')).not.toBeNull();
  });
});

describe('the tile claims nothing it has not established', () => {
  it('renders no NOT INDEXED badge, because nothing failed', () => {
    const container = drawCard();
    expect(container.textContent).not.toContain('NOT INDEXED');
  });

  it('renders no size and no file count — never a zero', () => {
    const strip = drawCard().querySelector('.cdt-strip');
    expect(strip).not.toBeNull();
    // §8.1's byte string is the only digit the strip can carry; an unmeasured inventory renders
    // no figure at all, and a `0 MB` here would read as an empty repository.
    expect(strip?.textContent).not.toMatch(/\d/);
    expect(strip?.textContent ?? '').not.toContain('0');
  });

  it('renders no freshness age slot and no stale marker', () => {
    const container = drawCard();
    const text = container.textContent ?? '';
    expect(text).not.toContain('as of');
    expect(text).not.toContain('stale');
    expect(text).not.toContain('ago');
    expect(text).not.toContain('no fetch recorded');
    // Every chip that carries an observation time is keyed on a working-copy fact, and every one
    // of those is null here, so band 4 carries no chip at all.
    expect(container.querySelector('.cdt-chip')).toBeNull();
  });

  it('bites: an observed ref state puts a chip with its clock back', () => {
    const container = drawCard({
      row: row({ refstateObservedAt: NOW - 60, ahead: 2 }),
    });
    expect(container.querySelector('.cdt-chip')).not.toBeNull();
  });

  it('renders no playtime figure', () => {
    const text = drawCard().textContent ?? '';
    expect(text).not.toContain('PLAYTIME');
    expect(text).not.toMatch(/\b0h\b/);
    expect(text).not.toContain('AT THE BENCH');
  });

  it('renders no tick row and keeps §7.7a‘s uncomputed rank', () => {
    const container = drawCard();
    expect(container.querySelector('.cdt-ticks')).toBeNull();
    expect(container.querySelector('.cdt-rank-label')?.textContent).toBe('NOT COMPUTED');
    expect(container.querySelector('.cdt-frame-gap')).not.toBeNull();
  });
});

/**
 * The flicker half, verified by **resolving a style on a real element** rather than by matching
 * the stylesheet's text: `motion.css`'s tier clamp selects nine exact class names, and a tenth
 * selects nothing while no gate says so.
 */
describe('§11.6: a project with no working copy never dips', () => {
  const haloOpacityAt = (tier: 'full' | 'reduced' | 'off'): string => {
    expect(cardCss.length, 'card.css imported as an empty string').toBeGreaterThan(0);
    expect(motionCss.length, 'motion.css imported as an empty string').toBeGreaterThan(0);
    const style = document.createElement('style');
    style.textContent = `${cardCss}\n${motionCss}`;
    document.head.append(style);
    document.documentElement.setAttribute('data-effects-tier', tier);
    const { container } = render(<ProjectCard {...cardProps({ haloOpacity: 0.8 })} />, {
      wrapper: withProjectDeps(),
    });
    const halo = container.querySelector('.cdt-card-halo');
    if (halo === null) throw new Error('the card mounted no .cdt-card-halo');
    return getComputedStyle(halo).opacity;
  };

  it('is not an eligible candidate at all, whatever the band says', () => {
    expect(flickerEligible(row())).toBe(false);
    expect(flickerEligible(row({ conditionSignal: 'neglected' }))).toBe(false);
    expect(flickerEligible(row({ conditionSignal: 'abandoned' }))).toBe(false);
    // The counter-case, so the assertion above is not vacuous.
    expect(
      flickerEligible({ conditionSignal: 'neglected', isReference: false, presence: 'present' }),
    ).toBe(true);
  });

  it('has a halo element the clamp actually selects, and the clamp wins at reduced and off', () => {
    // At `full` the halo's opacity is the custom property the driver writes — jsdom does not
    // resolve `var()`, so it reads back as the reference itself. At the two tiers that forbid
    // the dip the clamp replaces it with a literal `1`. If `.cdt-card-halo` were spelled
    // differently in motion.css the clamp would select nothing and all three would read the
    // reference — which is the defect this shape exists to catch, one level below a text match.
    const full = haloOpacityAt('full');
    expect(full).toContain('--cdt-halo-opacity');
    cleanup();
    expect(haloOpacityAt('reduced')).toBe('1');
    cleanup();
    expect(haloOpacityAt('off')).toBe('1');
  });
});

import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type {
  LocationId,
  ProjectId,
  ProjectRow,
  SceneHash,
  SessionId,
  SessionRef,
  TargetId,
} from '../../generated/protocol';
import motionCss from '../styles/motion.css?raw';
import { LADDER_RUNGS } from '../theme/tokens';
import { ProjectCard, type ProjectCardProps } from './ProjectCard';
import { withProjectDeps } from '../testing/deps';

afterEach(cleanup);

const MB = 1024 ** 2;

/** `#333c45` → `rgb(51, 60, 69)`, which is the only form an inline colour reaches the DOM in. */
const asRgb = (hex: string): string => {
  const n = Number.parseInt(hex.slice(1), 16);
  return `rgb(${String((n >> 16) & 255)}, ${String((n >> 8) & 255)}, ${String(n & 255)})`;
};

const row = (over: Partial<ProjectRow> = {}): ProjectRow => ({
  id: 1 as ProjectId,
  name: 'atlas',
  owner: null,
  description: 'a small tool',
  descriptionSource: null,
  birthYear: 2021,
  primaryLanguage: 'Rust',
  archetype: null,
  seedBasename: 'atlas',
  rerollOffset: 0,
  artSceneHash: 'aa' as SceneHash,
  artState: 'ready',
  conditionSignal: 'dormant',
  completionLit: null,
  completionApplicable: null,
  isPinned: false,
  isArchived: false,
  isHidden: false,
  isReference: false,
  isFork: false,
  isBare: false,
  isShallow: false,
  isSubmodule: false,
  ambiguousLineage: false,
  lastTouchedAt: 0,
  lastInteractionAt: null,
  lastCommitAt: null,
  lastCommitSubject: null,
  firstCommitAt: null,
  createdAt: 0,
  acknowledgedAt: null,
  sizeTrackedBytes: 100 * MB,
  trackedFiles: null,
  collectionIds: [],
  // §23.5 decides the frame from the pair, so a null location beside 'present' would draw the
  // blueprint frame on every case in this file — a fixture describing a state the product
  // cannot produce.
  primaryLocation: { id: 10 as LocationId, pathDisplay: '/w/atlas' },
  presence: 'present',
  hasRemote: false,
  branch: 'main',
  isDirty: null,
  untrackedCount: null,
  ahead: null,
  behind: null,
  stashCount: null,
  interruptedOp: null,
  fetchHeadAt: null,
  refstateObservedAt: null,
  worktreeObservedAt: null,
  errorKind: null,
  errorAt: null,
  eraSectionId: 'era-2021',
  ...over,
});

const session = (startedAt: number): SessionRef => ({
  id: 1 as SessionId,
  projectId: 1 as ProjectId,
  locationId: 1 as LocationId,
  targetId: 1 as TargetId,
  startedAt,
  endedAt: null,
  creditedSeconds: 0,
  closeReason: null,
});

const props = (over: Partial<ProjectCardProps> = {}): ProjectCardProps => ({
  row: row(),
  density: 186,
  rendition: 'card',
  selected: false,
  focused: false,
  now: 10_000,
  firstRunCompletedAt: null,
  session: null,
  haloOpacity: 1,
  onActivate: vi.fn(),
  onOpen: vi.fn(),
  onTogglePin: vi.fn(),
  onStopSession: vi.fn(),
  ...over,
});

const draw = (over: Partial<ProjectCardProps> = {}): HTMLElement =>
  render(<ProjectCard {...props(over)} />, { wrapper: withProjectDeps() }).container;

describe('band 5 carries the name, the identity line and the hover strip', () => {
  it('renders the name and the identity line, and no description at the default density', () => {
    const container = draw();
    expect(container.querySelector('.cdt-name')?.textContent).toBe('atlas');
    expect(container.querySelector('.cdt-identity')?.textContent).toContain('2021');
    expect(container.querySelector('.cdt-desc')).toBeNull();
  });

  it('shows the description only at the roomiest step', () => {
    expect(draw({ density: 240 }).querySelector('.cdt-desc')?.textContent).toBe('a small tool');
  });

  it('drops the identity line and the whole band-4 strip below 156px', () => {
    const container = draw({ density: 140 });
    expect(container.querySelector('.cdt-identity')).toBeNull();
    expect(container.querySelector('.cdt-chips')).toBeNull();
  });

  it('formats the hover strip‘s size through the one formatter', () => {
    expect(draw().querySelector('.cdt-strip')?.textContent).toContain('100 MB');
    // §8.1 switches to GB at 0.1 GB, which is 102.4 MiB — so a three-digit MB figure is one the
    // formatter cannot produce, and a card printing one is not going through it.
    expect(
      draw({ row: row({ sizeTrackedBytes: 200 * MB }) }).querySelector('.cdt-strip')?.textContent,
    ).toContain('0.2 GB');
  });

  it('renders no size at all when the inventory has not been measured', () => {
    const strip = draw({ row: row({ sizeTrackedBytes: null }) }).querySelector('.cdt-strip');
    expect(strip).not.toBeNull();
    expect(strip?.textContent).not.toMatch(/\d/);
  });

  it('omits the half of the identity line that is NULL and the line when both are', () => {
    expect(
      draw({ row: row({ birthYear: null, primaryLanguage: null }) }).querySelector('.cdt-identity'),
    ).toBeNull();
    expect(
      draw({ row: row({ birthYear: null }) }).querySelector('.cdt-identity')?.textContent,
    ).toBe('Rust');
  });
});

describe('§7.7a on the surface every phase-1 project is on', () => {
  it('draws the unknown frame, the gap and NOT COMPUTED on an ordinary project', () => {
    const container = draw();
    expect(
      container
        .querySelector<HTMLElement>('.cdt-card-frame')
        ?.style.getPropertyValue('--cdt-frame'),
    ).toBe('var(--unknown)');
    expect(container.querySelector('.cdt-frame-gap')).not.toBeNull();
    expect(container.querySelector('.cdt-rank-label')?.textContent).toBe('NOT COMPUTED');
  });

  it('takes the reference frame but keeps the gap, because completion is uncomputed there too', () => {
    const container = draw({ row: row({ isReference: true }) });
    expect(
      container
        .querySelector<HTMLElement>('.cdt-card-frame')
        ?.style.getPropertyValue('--cdt-frame'),
    ).toBe('var(--tier-ref)');
    expect(container.querySelector('.cdt-frame-gap')).not.toBeNull();
  });
});

describe('§7.8 hover is component state, and only this card re-renders', () => {
  it('marks itself hovered on mouse enter and clears it on leave', () => {
    const container = draw();
    const card = container.querySelector('.cdt-card');
    if (card === null) throw new Error('no card');
    fireEvent.mouseEnter(card);
    expect(card.getAttribute('data-hovered')).toBe('true');
    fireEvent.mouseLeave(card);
    expect(card.getAttribute('data-hovered')).toBe('false');
  });

  it('carries no :hover rule of its own — the state is the only mechanism', () => {
    expect(draw().innerHTML).not.toContain(':hover');
  });

  it('reveals the pin on hover and on focus, and keeps a pinned one always visible', () => {
    expect(draw().querySelector('.cdt-pin')?.getAttribute('data-visible')).toBe('false');
    expect(draw({ focused: true }).querySelector('.cdt-pin')?.getAttribute('data-visible')).toBe(
      'true',
    );
    expect(
      draw({ row: row({ isPinned: true }) })
        .querySelector('.cdt-pin')
        ?.getAttribute('data-visible'),
    ).toBe('true');
  });
});

describe('§7.8a: the pin is a real button that changes no order', () => {
  it('mounts a button with aria-pressed and calls back without playing', () => {
    const onTogglePin = vi.fn();
    const onActivate = vi.fn();
    draw({ onTogglePin, onActivate });
    const pin = screen.getByRole('button', { name: 'Pin atlas' });
    expect(pin.getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(pin);
    expect(onTogglePin).toHaveBeenCalledOnce();
    expect(onActivate).not.toHaveBeenCalled();
  });
});

describe('§7.8‘s live tile', () => {
  it('shows the permanent bench row and an explicit STOP when a session is open', () => {
    const container = draw({ session: session(10_000 - (2 * 3600 + 7 * 60)) });
    expect(container.querySelector('.cdt-bench')?.textContent).toContain('AT THE BENCH');
    expect(container.querySelector('.cdt-bench')?.textContent).toContain('2h 07m');
    expect(screen.getByRole('button', { name: /stop/i })).toBeTruthy();
  });

  it('renders no bench row and no timer with no open session', () => {
    expect(draw().querySelector('.cdt-bench')).toBeNull();
  });

  it('renders no bench row for a session that has already ended', () => {
    const ended = { ...session(10_000 - 600), endedAt: 10_000 - 60 };
    expect(draw({ session: ended }).querySelector('.cdt-bench')).toBeNull();
  });

  it('advances the figure when the shelf hands down a later instant', () => {
    // The row re-derives from `started_at` on every paint, so the caller's clock is what moves
    // it. A shelf that never advances `now` freezes this row and every as-of clause with it.
    const open = session(10_000 - 600);
    expect(draw({ session: open }).querySelector('.cdt-bench')?.textContent).toContain('10m');
    expect(
      draw({ session: open, now: 10_000 + 600 }).querySelector('.cdt-bench')?.textContent,
    ).toContain('20m');
  });

  it('never sums the figure into anything, and labels it', () => {
    const container = draw({ session: session(10_000 - 600) });
    expect(container.querySelector('.cdt-bench')?.textContent).not.toContain('total');
    expect(container.querySelector('.cdt-bench')?.textContent).not.toContain('playtime');
  });

  it('stops the session without playing it', () => {
    const onStopSession = vi.fn();
    const onActivate = vi.fn();
    draw({ session: session(10_000 - 600), onStopSession, onActivate });
    fireEvent.click(screen.getByRole('button', { name: /stop/i }));
    expect(onStopSession).toHaveBeenCalledOnce();
    expect(onActivate).not.toHaveBeenCalled();
  });
});

describe('§11.6: the dip arrives as a value chosen above the card', () => {
  it('passes the shelf‘s halo opacity through as a custom property, never as an opacity', () => {
    const container = draw({ haloOpacity: 0.8 });
    expect(
      container
        .querySelector<HTMLElement>('.cdt-card-frame')
        ?.style.getPropertyValue('--cdt-halo-opacity'),
    ).toBe('0.8');
    expect(container.querySelector<HTMLElement>('.cdt-card-halo')?.style.opacity).toBe('');
  });
});

describe('§11.7: the cell', () => {
  it('is a gridcell with a roving tabindex and no aria-label of its own', () => {
    const focused = draw({ focused: true }).querySelector('.cdt-card');
    expect(focused?.getAttribute('role')).toBe('gridcell');
    expect(focused?.getAttribute('tabindex')).toBe('0');
    expect(focused?.hasAttribute('aria-label')).toBe(false);
    expect(draw().querySelector('.cdt-card')?.getAttribute('tabindex')).toBe('-1');
  });

  it('plays on a click and opens the project page on Shift+click', () => {
    const onActivate = vi.fn();
    const onOpen = vi.fn();
    const container = draw({ onActivate, onOpen });
    const card = container.querySelector('.cdt-card');
    if (card === null) throw new Error('no card');
    fireEvent.click(card);
    expect(onActivate).toHaveBeenCalledOnce();
    expect(onOpen).not.toHaveBeenCalled();
    fireEvent.click(card, { shiftKey: true });
    expect(onOpen).toHaveBeenCalledOnce();
    expect(onActivate).toHaveBeenCalledOnce();
  });
});

/**
 * The tier clamp selects nine class names. Until this component existed it matched no element in
 * the product, and nothing said so — R36's shape one level down. This is the half that a rename
 * on either side breaks: what the clamp names, and what a real mounted tile carries.
 */
const UNMOUNTED = new Set(['.cdt-bracket']);

describe('the mounted tile carries the class names motion.css clamps', () => {
  it('mounts every clamped class except the ones nothing produces yet', () => {
    const clamped = [...new Set(motionCss.match(/\.cdt-[a-z-]+/g) ?? [])];
    expect(clamped.length).toBeGreaterThan(0);
    const container = draw({ row: row({ isPinned: true }) });
    for (const name of clamped) {
      if (UNMOUNTED.has(name)) continue;
      expect(
        container.querySelector(name),
        `${name} is clamped and mounted by nothing`,
      ).not.toBeNull();
    }
  });

  it('names the ones that really are absent, so the exception cannot grow unnoticed', () => {
    // §7.8 lists the corner brackets among its ten hover effects and `card.css` declares the
    // rule, but no section gives them a corner geometry, so this plan mounts none. If one is
    // ever mounted this fails and the exception above must shrink with it.
    const container = draw({ row: row({ isPinned: true }) });
    for (const name of UNMOUNTED) {
      expect(container.querySelector(name)).toBeNull();
    }
  });
});

describe('the invariants that have each been violated once', () => {
  it('renders no roast anywhere — the card‘s only input carries none', () => {
    expect(Object.keys(row())).not.toContain('roast');
  });

  it('renders no completion figure, no tick row and no ladder colour', () => {
    const html = draw().innerHTML;
    // Both spellings. A hex written into an inline style never reaches the DOM as a hex — jsdom
    // serialises `#333c45` back as `rgb(51, 60, 69)` — so a guard that reads only the hex form
    // passes against a card that really is painting a rung, which is how it was measured here.
    // The rungs come from `theme/tokens.ts`; retyping them is a second list to drift.
    for (const rung of LADDER_RUNGS) {
      expect(html, `${rung} is a completion rung`).not.toContain(rung);
      expect(html, `${rung} as rgb() is the same rung`).not.toContain(asRgb(rung));
    }
    expect(html).not.toMatch(/\b0\s*\/\s*10\b/);
    expect(draw().querySelectorAll('[data-tick]')).toHaveLength(0);
  });

  it('renders no BEHIND 0 and no chip from an absent fetch', () => {
    const html = draw({ row: row({ behind: 0, fetchHeadAt: null }) }).innerHTML;
    expect(html).not.toContain('BEHIND');
  });

  it('mounts no condition dot at all when nothing has measured the band', () => {
    expect(draw({ row: row({ conditionSignal: null }) }).querySelector('.cdt-dot')).toBeNull();
  });
});

/**
 * §43.2's band: when it renders, what it mirrors, and when it tells the core it was seen. It
 * stamps `bandShown` only on a transition into *on screen and the window active* (§43.9) — a
 * hidden or unfocused window sends nothing, and a band that does not render sends nothing.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { createRef, type ReactElement } from 'react';
import { afterEach, beforeEach, describe, expect, it, test, vi } from 'vitest';
import type {
  MotivationState,
  ProjectId,
  ProjectRow,
  RingStatus,
  SessionRef,
} from '../../generated/protocol';
import { DATA_MOTIVATION } from '../../shared/motivationQuiet';
import type { AppDeps } from '../app/deps';
import { BENCH_LABEL } from '../card/useBenchElapsed';
import bandCss from './band.css?raw';
import { LEVEL_LABEL, RHYTHM_LABEL, RING_LABELS, SHIP_OPEN_SUB, UNKNOWN } from './copy';
import { bandRenders, benchSession, StatusBand, type StatusBandProps } from './StatusBand';

/** jsdom has no IntersectionObserver: each instance records its target and fires on demand. */
class FakeObserver {
  static instances: FakeObserver[] = [];
  readonly root: Element | Document | null;
  readonly rootMargin = '';
  readonly thresholds: readonly number[] = [];
  private target: Element | null = null;
  private readonly callback: IntersectionObserverCallback;
  constructor(callback: IntersectionObserverCallback, options?: IntersectionObserverInit) {
    this.callback = callback;
    this.root = options?.root ?? null;
    FakeObserver.instances.push(this);
  }
  observe(target: Element): void {
    this.target = target;
  }
  unobserve(): void {
    this.target = null;
  }
  disconnect(): void {
    this.target = null;
  }
  takeRecords(): IntersectionObserverEntry[] {
    return [];
  }
  fire(visible: boolean): void {
    const target = this.target;
    if (target === null) return;
    const entry = { isIntersecting: visible, target } as unknown as IntersectionObserverEntry;
    this.callback([entry], this);
  }
}

const fireAll = (visible: boolean): void => {
  act(() => {
    for (const observer of FakeObserver.instances) observer.fire(visible);
  });
};

let visibility: DocumentVisibilityState = 'visible';

beforeEach(() => {
  FakeObserver.instances = [];
  vi.stubGlobal('IntersectionObserver', FakeObserver);
  visibility = 'visible';
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => visibility,
  });
  vi.spyOn(document, 'hasFocus').mockReturnValue(true);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const row = (id: number, over: Partial<ProjectRow> = {}): ProjectRow =>
  ({ id: id as ProjectId, name: `project-${String(id)}`, isHidden: false, ...over }) as ProjectRow;

const state = (over: Partial<MotivationState> = {}): MotivationState => ({
  shown: true,
  level: {
    state: 'computed',
    figures: { level: 4, xp: 500, levelAt: 400, nextLevelAt: 700, floored: false },
    pendingProjects: 0,
  },
  rings: {
    today: '2026-11-04',
    weekStart: '2026-11-02',
    waived: false,
    focus: { creditedSeconds: 600, targetSeconds: 2_700, status: 'open' },
    ship: { status: 'closed', pushedOn: '2026-11-03', observable: 1, unobservable: 0 },
    tend: { status: 'notComputed', excluded: [], unobservableProjects: 0 },
    rhythm: {
      days: ['active', 'active', 'pending', 'future', 'future', 'future', 'future'],
      activeDays: 2,
      target: 4,
      targetBasis: 'derived',
      runWeeks: 1,
      bestWeeks: 3,
      gitCoverage: 'complete',
    },
  },
  xpBar: 1 / 3,
  pendingLine: null,
  terms: null,
  ...over,
});

const session = (projectId: number, startedAt: number): SessionRef =>
  ({
    id: projectId * 10,
    projectId: projectId as ProjectId,
    startedAt,
    endedAt: null,
    creditedSeconds: 0,
  }) as unknown as SessionRef;

interface Drawn {
  readonly request: ReturnType<typeof vi.fn>;
  readonly onOpenProject: ReturnType<typeof vi.fn>;
  readonly onOpenProfile: ReturnType<typeof vi.fn>;
  readonly container: HTMLElement;
}

function draw(over: Partial<StatusBandProps> = {}): Drawn {
  const request = vi.fn(() => Promise.resolve(state()));
  const onOpenProject = vi.fn();
  const onOpenProfile = vi.fn();
  const scrollRef = createRef<HTMLDivElement>();
  const props: StatusBandProps = {
    route: 'shelf',
    state: state(),
    rows: [row(1)],
    sessions: new Map(),
    now: () => 1_000_000,
    onOpenProject,
    onOpenProfile,
    scrollRef,
    request: request as unknown as AppDeps['request'],
    ...over,
  };
  const { container } = render(
    <div ref={scrollRef}>
      <StatusBand {...props} />
    </div>,
  );
  return { request, onOpenProject, onOpenProfile, container };
}

const acks = (request: ReturnType<typeof vi.fn>): unknown[][] =>
  request.mock.calls.filter((call) => call[0] === 'motivation.acknowledge');

test('AC-P4-43-14 an unfocused or hidden window stamps nothing', () => {
  // Active and on screen: exactly one stamp, on the transition, not one per observation.
  const shown = draw();
  fireAll(true);
  fireAll(true);
  expect(acks(shown.request)).toEqual([
    ['motivation.acknowledge', { event: 'bandShown', level: null }],
  ]);
  cleanup();

  // On screen in a window without focus: nothing.
  vi.spyOn(document, 'hasFocus').mockReturnValue(false);
  const unfocused = draw();
  fireAll(true);
  expect(acks(unfocused.request)).toEqual([]);
  cleanup();

  // On screen in a hidden window: nothing.
  vi.spyOn(document, 'hasFocus').mockReturnValue(true);
  visibility = 'hidden';
  const hidden = draw();
  fireAll(true);
  expect(acks(hidden.request)).toEqual([]);

  // The window coming back to the foreground with the band on screen is the transition.
  visibility = 'visible';
  act(() => {
    document.dispatchEvent(new Event('visibilitychange'));
  });
  expect(acks(hidden.request)).toHaveLength(1);
});

describe('the status band', () => {
  it('renders only on the shelf, while shown, over a library with a live row', () => {
    const rows = [row(1)];
    expect(bandRenders({ route: 'shelf', state: state(), rows })).toBe(true);
    expect(bandRenders({ route: 'shelf', state: state({ shown: false }), rows })).toBe(false);
    expect(bandRenders({ route: 'shelf', state: null, rows })).toBe(false);
    expect(bandRenders({ route: 'shelf', state: state(), rows: [] })).toBe(false);
    expect(
      bandRenders({ route: 'shelf', state: state(), rows: [row(1, { isHidden: true })] }),
    ).toBe(false);
    for (const route of ['project', 'first-run', 'failure'] as const) {
      expect(bandRenders({ route, state: state(), rows })).toBe(false);
    }
    expect(draw({ state: state({ shown: false }) }).container.textContent).toBe('');
    cleanup();
    expect(draw({ route: 'project' }).container.textContent).toBe('');
  });

  it('draws nothing and stamps nothing over an empty library whose level reads computed', async () => {
    // An empty index reads `computed`; with no live row the band is not drawn, so nothing is
    // ever presented and day one and the shown level stay unwritten.
    const { container, request } = draw({ rows: [] });
    fireAll(true);
    await act(async () => {
      await Promise.resolve();
    });
    expect(container.querySelector('.cdt-band')).toBeNull();
    expect(container.querySelector(`[${DATA_MOTIVATION}]`)).toBeNull();
    expect(acks(request)).toEqual([]);
  });

  it('mirrors the most recently started open session, and is absent with none', () => {
    const none = draw();
    expect(screen.queryByText(new RegExp(BENCH_LABEL, 'u'))).toBeNull();
    expect(none.container.querySelector('.cdt-band-session')).toBeNull();
    cleanup();

    const sessions = new Map([
      [1 as ProjectId, session(1, 1_000_000 - 3_600)],
      [2 as ProjectId, session(2, 1_000_000 - 420)],
    ]);
    const live = draw({ rows: [row(1), row(2)], sessions });
    expect(screen.getByText('project-2')).toBeTruthy();
    expect(screen.getByText(`${BENCH_LABEL} · 7m`)).toBeTruthy();
    const block = live.container.querySelector<HTMLButtonElement>('button.cdt-band-session');
    if (block === null) throw new Error('the session block is not a button');
    fireEvent.click(block);
    expect(live.onOpenProject).toHaveBeenCalledWith(2);
  });

  it('picks the latest open session by its start', () => {
    const rows = [row(1), row(2)];
    const ended = { ...session(2, 900), endedAt: 950 } as SessionRef;
    expect(benchSession(new Map(), rows)).toBeNull();
    expect(benchSession(new Map([[2 as ProjectId, ended]]), rows)).toBeNull();
    const picked = benchSession(
      new Map([
        [1 as ProjectId, session(1, 500)],
        [2 as ProjectId, session(2, 400)],
      ]),
      rows,
    );
    expect(picked?.projectId).toBe(1);
  });

  it('makes the number a real button that opens Profile, and names every state in words', () => {
    const { onOpenProfile, container, request } = draw();
    const number = screen.getByRole('button', { name: `${LEVEL_LABEL} 4` });
    fireEvent.click(number);
    expect(onOpenProfile.mock.calls).toEqual([[{ section: 'level' }]]);
    // The number replays nothing and acknowledges nothing: it only opens Profile.
    expect(acks(request)).toEqual([]);
    const named = container.querySelectorAll('[role="img"][aria-label]');
    // Three rings and seven cells.
    expect(named).toHaveLength(10);
    for (const element of named) expect(element.getAttribute('aria-label')).not.toBe('');
    expect(screen.getByText(RHYTHM_LABEL)).toBeTruthy();
    for (const id of ['level', 'xp', 'rings', 'rhythm']) {
      expect(container.querySelector(`[${DATA_MOTIVATION}="${id}"]`), id).not.toBeNull();
    }
  });

  it('draws a not-computed ring as the --unknown glyph with U+2014, never an empty ring or a 0', () => {
    expect(bandCss.length, 'band.css imported as an empty string').toBeGreaterThan(0);
    // The fixture's Tend ring is notComputed; Focus and Ship are measured.
    const { container } = draw();
    const unknown = container.querySelectorAll('.cdt-band-ring-glyph[data-fill="unknown"]');
    expect(unknown).toHaveLength(1);
    const centre = unknown[0]?.querySelector('.cdt-band-ring-centre');
    expect(centre?.textContent).toBe(UNKNOWN);
    expect(centre?.getAttribute('data-unknown')).toBe('true');
    expect(unknown[0]?.closest('[role="img"]')?.getAttribute('aria-label')).not.toMatch(/\b0\b/u);
    // The attribute is drawn: band.css paints that fill with the --unknown token.
    const rule = /\.cdt-band-ring-glyph\[data-fill='unknown'\]\s*\{([^}]*)\}/u.exec(bandCss);
    expect(rule?.[1]).toMatch(/var\(--unknown\)/u);
  });

  it('says no push is recorded under an open Ship ring, and under no other state', () => {
    const statuses: RingStatus[] = [
      'open',
      'closed',
      'nothingToTend',
      'notApplicable',
      'notComputed',
    ];
    for (const status of statuses) {
      const base = state();
      const rings = base.rings;
      if (rings === null) throw new Error('the fixture has no rings');
      const { container } = draw({
        state: {
          ...base,
          rings: { ...rings, ship: { status, pushedOn: null, observable: 1, unobservable: 0 } },
        },
      });
      const ship = [...container.querySelectorAll('[role="img"]')].find((element) =>
        element.getAttribute('aria-label')?.startsWith(`${RING_LABELS.ship},`),
      );
      expect(ship, status).toBeDefined();
      const sub = ship?.querySelector('.cdt-band-ring-sub')?.textContent ?? null;
      const named = ship?.getAttribute('aria-label') ?? '';
      // The fixture's Focus ring is open too: the sub-line is Ship's alone.
      const everywhere = [...container.querySelectorAll('.cdt-band-ring-sub')].filter(
        (element) => element.textContent === SHIP_OPEN_SUB,
      );
      if (status === 'open') {
        expect(sub, status).toBe(SHIP_OPEN_SUB);
        expect(named, status).toContain(SHIP_OPEN_SUB);
        expect(everywhere, status).toHaveLength(1);
      } else {
        expect(sub, status).not.toBe(SHIP_OPEN_SUB);
        expect(named, status).not.toContain(SHIP_OPEN_SUB);
        expect(everywhere, status).toHaveLength(0);
      }
      cleanup();
    }
  });

  it('flips the number on the next key press after a rise, and announces nothing new', () => {
    // Levels no other test paints: the flip compares with the last level this renderer painted.
    const at = (
      level: number,
      pendingLine: MotivationState['pendingLine'] = null,
    ): MotivationState =>
      state({
        level: {
          state: 'computed',
          figures: { level, xp: 500, levelAt: 400, nextLevelAt: 700, floored: false },
          pendingProjects: 0,
        },
        pendingLine,
      });
    const scrollRef = createRef<HTMLDivElement>();
    const props: StatusBandProps = {
      route: 'shelf',
      state: at(40),
      rows: [row(1)],
      sessions: new Map(),
      now: () => 1_000_000,
      onOpenProject: vi.fn(),
      onOpenProfile: vi.fn(),
      scrollRef,
      request: vi.fn(() => Promise.resolve(state())) as unknown as AppDeps['request'],
    };
    const view = (over: Partial<StatusBandProps>): ReactElement => (
      <div ref={scrollRef}>
        <StatusBand {...props} {...over} />
      </div>
    );
    const { container, rerender } = render(view({}));
    fireAll(true);
    const digits = (): Element[] => [...container.querySelectorAll('.cdt-level-digit')];
    const flipping = (): number => container.querySelectorAll('[data-flip]').length;
    // At rest: one digit, nothing flipping.
    expect(digits()).toHaveLength(1);
    expect(flipping()).toBe(0);

    const rise = { form: 'rise', level: 41, reason: null, award: null } as const;
    rerender(view({ state: at(41, rise) }));
    // Data arriving moves nothing: the number changes in place and waits for the user.
    expect(digits().map((d) => d.textContent)).toEqual(['41']);
    expect(flipping()).toBe(0);

    const focused = document.activeElement;
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'j' }));
    });
    // The flip: the old digits leave, hidden from assistive technology, the new arrive, and the
    // fill surges under its flash.
    expect(digits().map((d) => [d.textContent, d.getAttribute('data-flip')])).toEqual([
      ['40', 'old'],
      ['41', 'new'],
    ]);
    expect(digits()[0]?.getAttribute('aria-hidden')).toBe('true');
    expect(container.querySelector(".cdt-xp-fill[data-flip='surge']")).not.toBeNull();
    expect(container.querySelector(".cdt-xp-fill[data-flip='flash']")).not.toBeNull();
    // What a screen reader is given is unchanged by it: the button's name is the level now, no
    // live region or alert exists, and focus has not moved.
    // A rise with no reason is the level alone, so the waiting line reads the same words: of the
    // buttons with that name, exactly one is the number.
    const named = screen.getAllByRole('button', { name: `${LEVEL_LABEL} 41` });
    expect(named.filter((b) => b.classList.contains('cdt-band-level-number'))).toHaveLength(1);
    expect(container.querySelector('[aria-live], [role="alert"], [role="status"]')).toBeNull();
    expect(document.activeElement).toBe(focused);

    // Its end leaves the end state: one digit, nothing flipping, and no second flip on input.
    const incoming = digits()[1];
    if (incoming === undefined) throw new Error('no incoming digit');
    fireEvent.animationEnd(incoming);
    expect(digits().map((d) => d.textContent)).toEqual(['41']);
    expect(flipping()).toBe(0);
    act(() => {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: 'j' }));
    });
    expect(flipping()).toBe(0);
  });

  it('has no live region and no transition on a ring, the bar or a cell', () => {
    expect(bandCss.length, 'band.css imported as an empty string').toBeGreaterThan(0);
    const style = document.createElement('style');
    style.textContent = bandCss;
    document.head.appendChild(style);
    try {
      const { container } = draw();
      expect(container.querySelector('[role="alert"]')).toBeNull();
      expect(container.querySelector('[aria-live]')).toBeNull();
      const moving = [
        ...container.querySelectorAll('.cdt-band-ring-glyph'),
        ...container.querySelectorAll('.cdt-band-xp-fill'),
        ...container.querySelectorAll('.cdt-band-cell'),
      ];
      expect(moving.length).toBe(3 + 1 + 7);
      for (const element of moving) {
        expect(getComputedStyle(element).transition, element.className).toBe('none');
      }
    } finally {
      style.remove();
    }
  });

  it('draws the line in the level block, and lands its badge only on screen in an active window', () => {
    const badgeLine: MotivationState['pendingLine'] = {
      form: 'badge',
      level: null,
      reason: null,
      award: {
        kind: 'marathon',
        earnedOn: '2026-11-03',
        projectId: null,
        evidence: {
          projectName: null,
          gapDays: null,
          year: null,
          languages: null,
          creditedSeconds: null,
          projects: null,
          runStartOn: null,
        },
        landed: false,
      },
    };
    const landings = (request: ReturnType<typeof vi.fn>): unknown[][] =>
      request.mock.calls.filter((call) => call[0] === 'badges.markLanded');

    // On screen in a window without focus: drawn, and nothing lands.
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    const unfocused = draw({ state: state({ pendingLine: badgeLine }) });
    fireAll(true);
    const line = unfocused.container.querySelector(`.cdt-band-level [${DATA_MOTIVATION}="line"]`);
    expect(line).not.toBeNull();
    expect(landings(unfocused.request)).toEqual([]);
    cleanup();

    // Off screen in an active window: nothing lands.
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const offScreen = draw({ state: state({ pendingLine: badgeLine }) });
    fireAll(false);
    expect(landings(offScreen.request)).toEqual([]);

    // Scrolled into view: it lands once.
    fireAll(true);
    expect(landings(offScreen.request)).toEqual([['badges.markLanded', { kind: 'marathon' }]]);
  });
});

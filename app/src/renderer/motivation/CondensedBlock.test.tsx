/**
 * §43.2's condensed top-bar block: what it draws, that a waiting line is text and never a dot,
 * that activating it reveals the band and acknowledges nothing, and that being on screen in an
 * active window is §43.5's `bandShown`. When it renders at all is the shelf's composition, and
 * `app/ShelfScreen.test.tsx` drives that through the band's real observer.
 */
import { cleanup, fireEvent, render } from '@testing-library/react';
import { createRef } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MotivationState, ProjectId, ProjectRow } from '../../generated/protocol';
import { DATA_MOTIVATION } from '../../shared/motivationQuiet';
import { required } from '../../shared/required';
import type { AppDeps } from '../app/deps';
import bandCss from './band.css?raw';
import { CondensedBlock, condensedModel, revealBand } from './CondensedBlock';
import { CONDENSED_LEVEL_LABEL, CONDENSED_LINE_WAITS, RING_LABELS, UNKNOWN } from './copy';
import { StatusBand } from './StatusBand';

let active = true;

beforeEach(() => {
  active = true;
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => 'visible',
  });
  vi.spyOn(document, 'hasFocus').mockImplementation(() => active);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

const state = (over: Partial<MotivationState> = {}): MotivationState => ({
  shown: true,
  level: {
    state: 'computed',
    figures: { level: 12, xp: 500, levelAt: 400, nextLevelAt: 700, floored: false },
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

const riseLine: MotivationState['pendingLine'] = {
  form: 'rise',
  level: 12,
  reason: null,
  award: null,
};

const requestFake = (): ReturnType<typeof vi.fn> => vi.fn(() => Promise.resolve(state()));
const asRequest = (fake: ReturnType<typeof vi.fn>): AppDeps['request'] =>
  fake as unknown as AppDeps['request'];

describe('condensedModel', () => {
  it('draws nothing under quiet mode', () => {
    expect(condensedModel(state({ shown: false, level: null, rings: null }))).toBeNull();
  });

  it('carries the level, three dots in §39’s order and the short rhythm', () => {
    const model = condensedModel(state());
    expect(model?.level).toBe('12');
    expect(model?.unknown).toBe(false);
    expect(model?.dots.map((dot) => dot.label)).toEqual([
      RING_LABELS.focus,
      RING_LABELS.ship,
      RING_LABELS.tend,
    ]);
    expect(model?.rhythm).toMatch(/2.*4.*3/u);
    expect(model?.waits).toBeNull();
  });

  it('draws the short rhythm’s active-day count as U+2014 while a day is not computed, never 0', () => {
    const base = state();
    if (base.rings === null) throw new Error('fixture carries rings');
    const model = condensedModel(
      state({
        rings: {
          ...base.rings,
          rhythm: {
            ...base.rings.rhythm,
            days: [
              'notComputed',
              'notComputed',
              'notComputed',
              'future',
              'future',
              'future',
              'future',
            ],
            activeDays: 0,
          },
        },
      }),
    );
    expect(model?.rhythm?.startsWith(UNKNOWN)).toBe(true);
    expect(model?.rhythm).not.toMatch(/\b0\b/u);
  });

  it('draws an uncomputed level as U+2014, never a zero', () => {
    const model = condensedModel(state({ level: null, xpBar: null }));
    expect(model?.level).toBe(UNKNOWN);
    expect(model?.unknown).toBe(true);
  });

  it('draws no dot on day one and nothing of the rings when the core sent none', () => {
    const base = state();
    if (base.rings === null) throw new Error('fixture carries rings');
    const waived = condensedModel(
      state({ rings: { ...base.rings, waived: true, focus: null, ship: null, tend: null } }),
    );
    expect(waived?.dots).toEqual([]);
    expect(waived?.rhythm).not.toBeNull();
    const none = condensedModel(state({ rings: null }));
    expect(none?.dots).toEqual([]);
    expect(none?.rhythm).toBeNull();
  });

  it('follows the level with the waits text only while a line is pending, and it holds no digit', () => {
    const model = condensedModel(state({ pendingLine: riseLine }));
    expect(model?.waits).toBe(CONDENSED_LINE_WAITS);
    expect(model?.waits).not.toMatch(/[0-9]/u);
  });
});

describe('CondensedBlock', () => {
  it('is one real button holding the level, the dots in words and the rhythm', () => {
    const { container } = render(
      <CondensedBlock state={state()} onReveal={vi.fn()} request={asRequest(requestFake())} />,
    );
    const buttons = container.querySelectorAll('button');
    expect(buttons).toHaveLength(1);
    const button = buttons[0];
    expect(button?.getAttribute('data-slot')).toBe('motivation');
    expect(button?.textContent).toContain(CONDENSED_LEVEL_LABEL);
    expect(button?.textContent).toContain('12');
    const dots = container.querySelectorAll('[role="img"][aria-label]');
    expect(dots).toHaveLength(3);
    for (const dot of dots) expect(dot.getAttribute('aria-label')).not.toBe('');
    for (const id of ['level', 'rings', 'rhythm']) {
      expect(container.querySelector(`[${DATA_MOTIVATION}="${id}"]`), id).not.toBeNull();
    }
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container.querySelector('[aria-live]')).toBeNull();
  });

  it('says a line waits in text after the level, never with a dot', () => {
    const plain = render(
      <CondensedBlock state={state()} onReveal={vi.fn()} request={asRequest(requestFake())} />,
    );
    const before = plain.container.querySelectorAll('button *').length;
    expect(plain.container.textContent).not.toContain(CONDENSED_LINE_WAITS);
    cleanup();

    const { container } = render(
      <CondensedBlock
        state={state({ pendingLine: riseLine })}
        onReveal={vi.fn()}
        request={asRequest(requestFake())}
      />,
    );
    const waits = container.querySelector(`[${DATA_MOTIVATION}="line"]`);
    expect(waits?.textContent).toBe(CONDENSED_LINE_WAITS);
    // One element more — the text — and still exactly the three ring dots.
    expect(container.querySelectorAll('button *').length).toBe(before + 1);
    expect(container.querySelectorAll('[role="img"]')).toHaveLength(3);
    // It follows the level.
    const level = container.querySelector(`[${DATA_MOTIVATION}="level"]`);
    expect(level?.nextElementSibling).toBe(waits);
  });

  it('reveals the band when activated and acknowledges nothing', () => {
    const request = requestFake();
    const onReveal = vi.fn();
    const { container } = render(
      <CondensedBlock
        state={state({ pendingLine: riseLine })}
        onReveal={onReveal}
        request={asRequest(request)}
      />,
    );
    const before = request.mock.calls.length;
    fireEvent.click(required(container.querySelector('button'), 'the block'));
    expect(onReveal).toHaveBeenCalledOnce();
    expect(request.mock.calls.length).toBe(before);
  });

  it('counts as bandShown when it paints in an active window, and only then', () => {
    const request = requestFake();
    render(<CondensedBlock state={state()} onReveal={vi.fn()} request={asRequest(request)} />);
    expect(request.mock.calls).toEqual([
      ['motivation.acknowledge', { event: 'bandShown', level: null }],
    ]);
    cleanup();

    active = false;
    const idle = requestFake();
    render(<CondensedBlock state={state()} onReveal={vi.fn()} request={asRequest(idle)} />);
    expect(idle.mock.calls).toEqual([]);
  });

  it('declares every class it applies, and nothing in it moves at rest', () => {
    expect(bandCss.length, 'band.css imported as an empty string').toBeGreaterThan(0);
    const style = document.createElement('style');
    style.textContent = bandCss;
    document.head.appendChild(style);
    try {
      const { container } = render(
        <CondensedBlock
          state={state({ pendingLine: riseLine })}
          onReveal={vi.fn()}
          request={asRequest(requestFake())}
        />,
      );
      const elements = [...container.querySelectorAll('[class]')];
      expect(elements.length).toBeGreaterThan(5);
      const declared = new Set(
        [...bandCss.matchAll(/\.(cdt-[A-Za-z0-9_-]+)/gu)].map(([, name]) => name),
      );
      for (const element of elements) {
        for (const name of element.classList) expect(declared.has(name), name).toBe(true);
        const computed = getComputedStyle(element);
        expect(computed.animationName || 'none', element.className).toBe('none');
      }
    } finally {
      style.remove();
    }
  });
});

describe('revealBand', () => {
  const row = { id: 1 as ProjectId, name: 'project-1', isHidden: false } as ProjectRow;

  function shelf(over: Partial<MotivationState> = {}): HTMLElement {
    const scrollRef = createRef<HTMLDivElement>();
    const { container } = render(
      <div ref={scrollRef} data-testid="scroll">
        <StatusBand
          route="shelf"
          state={state(over)}
          rows={[row]}
          sessions={new Map()}
          now={() => 1_000_000}
          onOpenProject={vi.fn()}
          onOpenProfile={vi.fn()}
          scrollRef={scrollRef}
          request={asRequest(requestFake())}
        />
      </div>,
    );
    const root = container.querySelector<HTMLElement>('[data-testid="scroll"]');
    if (root === null) throw new Error('no scroll container');
    root.scrollTop = 480;
    return root;
  }

  it('scrolls the band into view and moves focus to the waiting line', () => {
    const root = shelf({ pendingLine: riseLine });
    revealBand(root);
    expect(root.scrollTop).toBe(0);
    const focused = document.activeElement;
    expect(focused?.getAttribute(DATA_MOTIVATION)).toBe('line');
    expect(focused?.tagName).toBe('BUTTON');
  });

  it('moves focus to the level number when no line waits', () => {
    const root = shelf();
    revealBand(root);
    expect(root.scrollTop).toBe(0);
    expect(document.activeElement?.classList.contains('cdt-band-level-number')).toBe(true);
  });

  it('does nothing before the scroll container mounts', () => {
    expect(() => {
      revealBand(null);
    }).not.toThrow();
  });
});

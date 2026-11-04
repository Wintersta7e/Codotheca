/**
 * §43.6's flip: armed only by a rise the band has not yet painted in this renderer's lifetime,
 * fired only by the user, played at most once per pending level, and ended at its end state by
 * any input during it. The stylesheet half is read from the imported `band.css` and compared as
 * numbers, because the auto-formatter rewrites the literals.
 */
import { act, type RenderHookResult, renderHook } from '@testing-library/react';
import { describe, expect, it, test } from 'vitest';
import type { MotivationLine, MotivationState } from '../../generated/protocol';
import bandCss from './band.css?raw';
import { FLIP_INITIAL, type FlipEvent, flipReducer, type FlipState } from './flip';
import { type FlipLifetime, type LevelFlip, useLevelFlip } from './useLevelFlip';

type Mounted = RenderHookResult<LevelFlip, { s: MotivationState; on: boolean }>;

const run = (events: readonly FlipEvent[], from: FlipState = FLIP_INITIAL): FlipState =>
  events.reduce(flipReducer, from);

const painted = (level: number | null): FlipEvent => ({ kind: 'painted', level });
const changed = (level: number | null, form: MotivationLine['form'] | null): FlipEvent => ({
  kind: 'changed',
  level,
  form,
});
const input: FlipEvent = { kind: 'input' };
const ended: FlipEvent = { kind: 'ended' };

test('AC-P4-43-8 a rise above the painted level flips once on the next input, and nothing else arms it', () => {
  // A first paint at 7, then a rise to 8: armed, and nothing plays until the user acts.
  const armed = run([painted(7), changed(8, 'rise'), painted(8)]);
  expect(armed.armed).toEqual({ from: 7, to: 8 });
  expect(armed.playing).toBeNull();

  // The next input fires it, once.
  const fired = flipReducer(armed, input);
  expect(fired.playing).toEqual({ from: 7, to: 8 });
  expect(fired.armed).toBeNull();
  const done = flipReducer(fired, ended);
  expect(done.playing).toBeNull();
  expect(flipReducer(done, input)).toEqual(done);

  // At most one response per pending level: the same rise sent again does not arm again.
  expect(run([changed(8, 'rise'), painted(8)], done).armed).toBeNull();

  // An input mid-flip ends it at its end state, and does not start another.
  const cut = flipReducer(fired, input);
  expect(cut.playing).toBeNull();
  expect(cut.armed).toBeNull();

  // `arrival`, `return` and `badge` never arm, whatever their level.
  for (const form of ['arrival', 'return', 'badge'] as const) {
    expect(run([painted(7), changed(8, form), painted(8)]).armed, form).toBeNull();
    expect(run([painted(7), changed(8, form), painted(8), input]).playing, form).toBeNull();
  }
  // A silent rise — the number moves with no line — never arms.
  expect(run([painted(7), changed(null, null), painted(8), input]).playing).toBeNull();
  // A line that leaves for another form takes the armed rise with it.
  expect(run([painted(7), changed(8, 'rise'), changed(null, 'badge'), input]).playing).toBeNull();

  // A first paint at 8 with the rise already pending never plays: nothing was painted before it.
  const first = run([changed(8, 'rise'), painted(8)]);
  expect(first.armed).toBeNull();
  expect(flipReducer(first, input).playing).toBeNull();

  // A rise at or below the painted level is not a raise.
  expect(run([painted(8), changed(8, 'rise')]).armed).toBeNull();
  expect(run([painted(8), changed(7, 'rise')]).armed).toBeNull();
  // A number the band never drew — the level not computed — is not a level painted.
  expect(run([painted(null), changed(8, 'rise')]).armed).toBeNull();

  // A higher rise before the input keeps the start the user last saw.
  expect(run([painted(7), changed(8, 'rise'), painted(8), changed(9, 'rise')]).armed).toEqual({
    from: 7,
    to: 9,
  });
});

/** The one rule whose whole selector is `selector`, by its body's declarations. */
function ruleBody(css: string, selector: string): string {
  for (const match of css.matchAll(/([^{}]+)\{([^{}]*)\}/gu)) {
    const selectors = (match[1] ?? '').split(',').map((s) => s.trim().replace(/"/gu, "'"));
    if (selectors.includes(selector)) return match[2] ?? '';
  }
  throw new Error(`band.css has no rule for ${selector}`);
}

interface Envelope {
  readonly durationMs: number;
  readonly delayMs: number;
  readonly curve: readonly number[] | string;
}

/** An `animation` shorthand's duration, delay and curve, as numbers. */
function envelope(body: string): Envelope {
  const value = /(?:^|;)\s*animation\s*:\s*([^;]+)/u.exec(body)?.[1];
  if (value === undefined) throw new Error(`no animation in ${body}`);
  const times = [...value.matchAll(/(-?[\d.]+)(ms|s)\b/gu)].map(([, n, unit]) =>
    unit === 's' ? Number(n) * 1000 : Number(n),
  );
  const bezier = /cubic-bezier\(([^)]*)\)/u.exec(value)?.[1];
  const curve =
    bezier === undefined
      ? (/\b(ease-out|ease-in|ease|linear)\b/u.exec(value)?.[1] ?? '')
      : bezier.split(',').map(Number);
  return { durationMs: times[0] ?? Number.NaN, delayMs: times[1] ?? 0, curve };
}

test("AC-P4-43-8-css the flip's durations, delays and curves are §43.6's table, read as numbers", () => {
  expect(bandCss.length, 'band.css imported as an empty string').toBeGreaterThan(0);
  const exit = [0.4, 0.1, 0.6, 1];
  const standard = [0.2, 0.85, 0.2, 1];
  const table: readonly [string, Envelope][] = [
    [".cdt-level-digit[data-flip='old']", { durationMs: 500, delayMs: 100, curve: exit }],
    [".cdt-level-digit[data-flip='new']", { durationMs: 620, delayMs: 300, curve: standard }],
    [".cdt-xp-fill[data-flip='surge']", { durationMs: 800, delayMs: 0, curve: standard }],
    [".cdt-xp-fill[data-flip='flash']", { durationMs: 700, delayMs: 200, curve: 'ease-out' }],
  ];
  for (const [selector, expected] of table) {
    expect(envelope(ruleBody(bandCss, selector)), selector).toEqual(expected);
  }
  // The level-up panel's overshoot is not the flip's: its ban stands.
  expect(bandCss).not.toMatch(/cubic-bezier\(\s*0?\.2\s*,\s*1\.5\s*,/u);
});

const line = (form: MotivationLine['form'], level: number): MotivationLine => ({
  form,
  level,
  reason: null,
  award: null,
});

const reading = (level: number, pendingLine: MotivationLine | null = null): MotivationState => ({
  shown: true,
  level: {
    state: 'computed',
    figures: { level, xp: 500, levelAt: 400, nextLevelAt: 700, floored: false },
    pendingProjects: 0,
  },
  rings: null,
  xpBar: 0.5,
  pendingLine,
  terms: null,
});

const press = (): void => {
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'a' }));
};

describe('useLevelFlip', () => {
  const mount = (lifetime: FlipLifetime, state: MotivationState, onScreen = true): Mounted =>
    renderHook(({ s, on }) => useLevelFlip(s, on, lifetime), {
      initialProps: { s: state, on: onScreen },
    });

  it('fires on a key press, a pointer press or the window gaining focus, with the band on screen', () => {
    for (const fire of [
      press,
      (): void => {
        window.dispatchEvent(new Event('pointerdown'));
      },
      (): void => {
        window.dispatchEvent(new FocusEvent('focus'));
      },
    ]) {
      const lifetime: FlipLifetime = { current: FLIP_INITIAL };
      const hook = mount(lifetime, reading(7));
      hook.rerender({ s: reading(8, line('rise', 8)), on: true });
      expect(hook.result.current.playing).toBeNull();
      act(fire);
      expect(hook.result.current.playing).toEqual({ from: 7, to: 8 });
      act(() => {
        hook.result.current.end();
      });
      expect(hook.result.current.playing).toBeNull();
      hook.unmount();
    }
  });

  it('waits while the band is off screen, and fires once it is back', () => {
    const lifetime: FlipLifetime = { current: FLIP_INITIAL };
    const hook = mount(lifetime, reading(7), false);
    hook.rerender({ s: reading(8, line('rise', 8)), on: false });
    act(press);
    expect(hook.result.current.playing).toBeNull();
    hook.rerender({ s: reading(8, line('rise', 8)), on: true });
    act(press);
    expect(hook.result.current.playing).toEqual({ from: 7, to: 8 });
    hook.unmount();
  });

  it("keeps the renderer's lifetime across the band's remount, and a new lifetime flips nothing", () => {
    const lifetime: FlipLifetime = { current: FLIP_INITIAL };
    const shelf = mount(lifetime, reading(7));
    shelf.unmount();
    // The rise arrives while the band is not mounted; the band's next mount still arms it.
    const back = mount(lifetime, reading(8, line('rise', 8)));
    act(press);
    expect(back.result.current.playing).toEqual({ from: 7, to: 8 });
    back.unmount();
    // A reload: the first paint, with the same rise pending, plays nothing.
    const reload = mount({ current: FLIP_INITIAL }, reading(8, line('rise', 8)));
    act(press);
    expect(reload.result.current.playing).toBeNull();
    reload.unmount();
  });

  it('attaches no input listener while nothing is armed', () => {
    const added: string[] = [];
    const original = window.addEventListener.bind(window);
    window.addEventListener = ((type: string, ...rest: [never, never?]) => {
      added.push(type);
      original(type, ...rest);
    }) as typeof window.addEventListener;
    try {
      const hook = mount({ current: FLIP_INITIAL }, reading(7));
      hook.rerender({ s: reading(7, line('arrival', 7)), on: true });
      expect(added).toEqual([]);
      hook.unmount();
    } finally {
      window.addEventListener = original;
    }
  });
});

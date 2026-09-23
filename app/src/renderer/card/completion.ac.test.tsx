import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { LocationId, ProjectId, ProjectRow, SceneHash } from '../../generated/protocol';
import cardCss from '../styles/card.css?raw';
import motionCss from '../styles/motion.css?raw';
import transitionCss from '../styles/transition.css?raw';
import { withProjectDeps } from '../testing/deps';
import { TOKENS, type TokenName } from '../theme/tokens';
import { ProjectCard, type ProjectCardProps } from './ProjectCard';
import { paintsLadderRung, rungFor } from './completion';

/**
 * §31's render criteria for the card frame: **`AC-P3-31-5`**, **`AC-P3-31-7`**,
 * **`AC-P3-31-14`** and **`AC-P3-31-3`'s render half**.
 *
 * Every frame assertion here resolves the custom property **off a really mounted card** and maps
 * it back through the token map, rather than matching stylesheet text. A name in a stylesheet is
 * not proof the element carries it.
 */

afterEach(cleanup);

const MB = 1024 ** 2;

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
  // [p3] §30.1: `absent` is the reading nothing has computed, and every quantity is null.
  healthSummary: {
    state: 'absent',
    scoredOpen: null,
    unverified: null,
    unknownChecks: null,
    observedAt: null,
  },
  lifecycle: 'active',
  ...over,
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

const draw = (over: Partial<ProjectRow> = {}): HTMLElement =>
  render(<ProjectCard {...props({ row: row(over) })} />, { wrapper: withProjectDeps() }).container;

/**
 * The frame colour a mounted card actually resolves.
 *
 * `--cdt-frame` reaches the DOM as `var(--tier-gold)`; this reads it off the element and maps the
 * name back through the token map, so what is asserted is the value the card paints and not a
 * string that happens to appear in a stylesheet.
 */
function resolvedFrame(container: HTMLElement): string {
  const frame = container.querySelector<HTMLElement>('.cdt-card-frame');
  if (frame === null) throw new Error('no card frame');
  const raw = getComputedStyle(frame).getPropertyValue('--cdt-frame').trim();
  const name = /^var\(--([a-z0-9-]+)\)$/iu.exec(raw)?.[1];
  if (name === undefined) throw new Error(`--cdt-frame is not a token reference: ${raw}`);
  const value = TOKENS[name as TokenName] as string | undefined;
  if (value === undefined) throw new Error(`--cdt-frame names no token: ${name}`);
  return value;
}

describe('AC-P3-31-5: the notch fires on evaluable, not on the N/A count', () => {
  it('renders gold WITH a notch at 8/8 with two unknown, never plain gold', () => {
    // The design's own live fixture row — eight pass, zero fail, zero na, two unknown — which
    // the prototype renders as plain gold. The transcription is deliberately not faithful here.
    const container = draw({ completionLit: 8, completionApplicable: 8 });
    expect(resolvedFrame(container)).toBe(TOKENS['tier-gold']);
    expect(container.querySelector('.cdt-frame-notch')).not.toBeNull();
    // And never beside §7.7a's gap, which says there is no measurement at all.
    expect(container.querySelector('.cdt-frame-gap')).toBeNull();
  });

  it('renders plain gold at 10/10, because ten of ten needs no qualifier', () => {
    const container = draw({ completionLit: 10, completionApplicable: 10 });
    expect(resolvedFrame(container)).toBe(TOKENS['tier-gold']);
    expect(container.querySelector('.cdt-frame-notch')).toBeNull();
  });

  it('notches archived gold too, and paints its own frame', () => {
    const container = draw({
      completionLit: 7,
      completionApplicable: 7,
      isArchived: true,
    });
    expect(resolvedFrame(container)).toBe(TOKENS['tier-gold-archived']);
    expect(container.querySelector('.cdt-frame-notch')).not.toBeNull();
  });
});

describe('AC-P3-31-10: a content-scan budget exceedance is unknown, not absent', () => {
  it('ac_p3_31_10_not_read_keeps_seven_of_seven_gold_notched_while_absent_demotes', () => {
    // The core projection has seven passing checks. The three content checks are excluded from
    // its denominator when their enumeration reports not_read, so the card must qualify 7/7
    // with a notch. When those same checks are absent, all three fail and ciGreen becomes N/A;
    // the frame must reflect the resulting 6/9 measurement.
    const notRead = draw({ completionLit: 7, completionApplicable: 7 });
    expect(resolvedFrame(notRead)).toBe(TOKENS['tier-gold']);
    expect(notRead.querySelector('.cdt-frame-notch')).not.toBeNull();

    cleanup();
    const absent = draw({ completionLit: 6, completionApplicable: 9 });
    expect(resolvedFrame(absent)).toBe(TOKENS['tier-steel']);
    expect(absent.querySelector('.cdt-frame-notch')).toBeNull();
  });
});

describe('AC-P3-31-7: no surface paints a ladder rung while completion_lit is NULL', () => {
  it('paints no rung on the permanently-NotComputed fixture', () => {
    // §31.10's fixture, and the one §16's criteria 45a–45c need to stay testable once they are
    // scoped from *every row* to *`completion_lit IS NULL`*.
    const reference = draw({ isReference: true });
    expect(paintsLadderRung(resolvedFrame(reference))).toBe(false);
    expect(resolvedFrame(reference)).toBe(TOKENS['tier-ref']);

    const uncloned = draw({ primaryLocation: null });
    expect(paintsLadderRung(resolvedFrame(uncloned))).toBe(false);
    expect(resolvedFrame(uncloned)).toBe(TOKENS['tier-blue']);

    const unmeasured = draw();
    expect(paintsLadderRung(resolvedFrame(unmeasured))).toBe(false);
    expect(resolvedFrame(unmeasured)).toBe(TOKENS.unknown);
  });

  it('keeps the guard false for the two frames decided above the ladder', () => {
    expect(paintsLadderRung(TOKENS['tier-ref'])).toBe(false);
    expect(paintsLadderRung(TOKENS['tier-blue'])).toBe(false);
  });

  it('paints a rung exactly when a measurement exists', () => {
    let painted = 0;
    for (const [lit, evaluable] of [
      [10, 10],
      [9, 10],
      [6, 10],
      [1, 10],
    ] as const) {
      const container = draw({ completionLit: lit, completionApplicable: evaluable });
      expect(paintsLadderRung(resolvedFrame(container))).toBe(true);
      painted += 1;
      cleanup();
    }
    console.error(`completion.ac: resolved ${String(painted)} painted rung(s)`);
    expect(painted, 'a run that painted no rung proves nothing').toBeGreaterThan(0);
  });
});

describe('AC-P3-31-3: zero evaluable renders the uncomputed treatment, from the row', () => {
  it('draws §7.7a in full although the detail would carry ten rows', () => {
    // The store writes ten rows and NULLs the projection; the card reads the projection. That
    // pairing is the whole of the case — the checklist behind the HEALTH tab still renders all
    // ten with their reasons, and it reads `CompletionDetail`, never these scalars.
    const container = draw({ completionLit: null, completionApplicable: null });
    expect(container.querySelector('.cdt-frame-gap')).not.toBeNull();
    expect(container.querySelector('.cdt-frame-notch')).toBeNull();
    expect(container.querySelector('.cdt-rank-label')?.textContent).toBe('NOT COMPUTED');
    expect(paintsLadderRung(resolvedFrame(container))).toBe(false);
  });
});

/**
 * **`AC-P3-31-14`'s renderer half.** jsdom runs no CSS animation and fires no `animationstart`,
 * so a listener alone asserts nothing: what is asserted is that **the same mounted frame**,
 * re-rendered from a higher tier to a lower one, resolves no animation and gains no attribute or
 * class a stylesheet could animate it by — at `full`, the one tier at which frame animations
 * exist. The core half, *no `health_delta` row*, is `core/tests/acceptance_completion.rs`.
 */
describe('AC-P3-31-14: a demotion is silent', () => {
  const SHEETS = [
    ['card.css', cardCss],
    ['transition.css', transitionCss],
    ['motion.css', motionCss],
  ] as const;

  function withFrameStyles(): () => void {
    for (const [name, css] of SHEETS) {
      expect(css.length, `${name}?raw imported as an empty string`).toBeGreaterThan(0);
    }
    const style = document.createElement('style');
    style.textContent = SHEETS.map(([, css]) => css).join('\n');
    document.head.append(style);
    document.documentElement.setAttribute('data-effects-tier', 'full');
    return () => {
      style.remove();
      document.documentElement.removeAttribute('data-effects-tier');
    };
  }

  /**
   * The keyframe names a resolved `animation` runs; `[]` is none. Split on top-level commas only,
   * because a timing function carries commas of its own.
   */
  function animationsOn(element: Element): string[] {
    const value = getComputedStyle(element).animation;
    if (value === '' || /^none\b/u.test(value)) return [];
    const names: string[] = [];
    let depth = 0;
    let start = 0;
    for (let i = 0; i <= value.length; i += 1) {
      const c = value[i];
      if (c === '(') depth += 1;
      else if (c === ')') depth -= 1;
      else if ((c === ',' && depth === 0) || c === undefined) {
        names.push(value.slice(start, i).trim().split(/\s+/u)[0] ?? '');
        start = i + 1;
      }
    }
    return names;
  }

  it('fires no animation on the frame when the tier drops', () => {
    const restore = withFrameStyles();
    try {
      // The control: this cascade does animate this element when something asks it to, so the
      // empty result below is about the demotion and not a sheet jsdom never applied.
      const gestured = render(
        <ProjectCard
          {...props({ row: row({ completionLit: 7, completionApplicable: 7 }), gesture: 'unfold' })}
        />,
        { wrapper: withProjectDeps() },
      ).container.querySelector('.cdt-card-frame');
      if (gestured === null) throw new Error('no frame');
      expect(animationsOn(gestured)).toEqual(['cardUnfold']);
      cleanup();

      // Notched gold 7/7 → brass 8/9: connecting an account widened the denominator faster than
      // the numerator, which is a measurement changing and not an earned thing being removed.
      const { container, rerender } = render(
        <ProjectCard {...props({ row: row({ completionLit: 7, completionApplicable: 7 }) })} />,
        { wrapper: withProjectDeps() },
      );
      const frame = container.querySelector<HTMLElement>('.cdt-card-frame');
      if (frame === null) throw new Error('no frame');
      expect(resolvedFrame(container)).toBe(TOKENS['tier-gold']);
      const started: string[] = [];
      frame.addEventListener('animationstart', (e) => {
        started.push(e.animationName);
      });
      const classes = frame.className;
      const attributes = frame.getAttributeNames().sort();
      expect(animationsOn(frame)).toEqual([]);

      rerender(
        <ProjectCard {...props({ row: row({ completionLit: 8, completionApplicable: 9 }) })} />,
      );
      // The same element, repainted: a remount would reset any animation rather than show one.
      expect(container.querySelector('.cdt-card-frame')).toBe(frame);
      expect(resolvedFrame(container)).toBe(TOKENS['tier-brass']);
      expect(
        rungFor({
          completionLit: 8,
          completionApplicable: 9,
          isReference: false,
          hasWorkingCopy: true,
          isArchived: false,
        })?.rung,
      ).toBe('brass');

      expect(animationsOn(frame), 'the demoted frame resolves an animation').toEqual([]);
      expect(frame.getAttribute('data-gesture')).toBeNull();
      expect(frame.className).toBe(classes);
      expect(frame.getAttributeNames().sort()).toEqual(attributes);
      expect(started).toHaveLength(0);
      // No gap appears either: the measurement is still a measurement.
      expect(container.querySelector('.cdt-frame-gap')).toBeNull();
      expect(container.querySelector('.cdt-frame-notch')).toBeNull();
    } finally {
      restore();
    }
  });
});

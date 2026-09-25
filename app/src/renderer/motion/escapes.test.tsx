/**
 * §11.6's clamp, **resolved on real mounted elements**, for the six animated classes the standing
 * clamp checker (`scripts/check-motion-clamp.mjs`) first found unclamped: the first run's entries
 * on `.cdt-fr-view`, `--scan`, `--turn` and `.cdt-fr-evidence`, the sigil's hover transform, and
 * the rescan line's travelling band.
 *
 * At `reduced`: opacity or colour only, no longer than `REDUCED_CLAMP_MS`, no transform, no
 * travelling highlight. At `off`: nothing moves. Every tier is checked against a `full` control
 * that does move, or the lower tiers would pass over a stylesheet jsdom never applied.
 *
 * No assertion matches stylesheet text and every duration is compared as a number: the write
 * hook reformats `.css`, and `.26s` may come back as `0.26s`.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import type { ProjectId, Reveal, RevealBasis } from '../../generated/protocol';
import { CardPlate } from '../card/CardPlate';
import firstRunCss from '../firstrun/firstRun.css?raw';
import { RescanLine } from '../firstrun/RescanLine';
import { RevealScreen } from '../firstrun/RevealScreen';
import type { RevealDeps } from '../firstrun/revealModel';
import { ScanScreen } from '../firstrun/ScanScreen';
import { INITIAL_SCAN_FEED, scanFeedReducer, type ScanFeedEvent } from '../firstrun/scanFeed';
import { TurnScreen } from '../firstrun/TurnScreen';
import cardCss from '../styles/card.css?raw';
import motionCss from '../styles/motion.css?raw';
import { REDUCED_CLAMP_MS, type ResolvedTier } from './tier';
import { noop } from '../noop';

const TIERS = ['full', 'reduced', 'off'] as const;

beforeEach(() => {
  // Every sheet non-empty FIRST: a `?raw` import stubbed to "" would resolve every assertion
  // below against an empty cascade and pass. The product's import order: card, first run, motion.
  for (const [name, css] of [
    ['card.css', cardCss],
    ['firstRun.css', firstRunCss],
    ['motion.css', motionCss],
  ] as const) {
    expect(css.length, `${name}?raw imported as an empty string`).toBeGreaterThan(0);
  }
  const style = document.createElement('style');
  style.textContent = `${cardCss}\n${firstRunCss}\n${motionCss}`;
  document.head.append(style);
});

afterEach(() => {
  cleanup();
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
});

/** The tier on the document element, as `useTierOnDocument` sets it in the product. */
function atTier(tier: ResolvedTier): void {
  cleanup();
  document.documentElement.setAttribute('data-effects-tier', tier);
}

/** The first time value in a resolved value, in milliseconds, **as a number**. */
function firstMs(value: string): number | null {
  const m = /(?<![\w.-])(\d*\.?\d+)(ms|s)\b/u.exec(value);
  if (m === null) return null;
  const amount = Number.parseFloat(m[1] ?? '0');
  return m[2] === 's' ? amount * 1000 : amount;
}

interface Entry {
  readonly name: string | null;
  readonly ms: number | null;
}

/** The entry keyframe a resolved `animation` runs, and for how long; `null` name is none. */
function entryOf(element: Element): Entry {
  const value = getComputedStyle(element).animation;
  const name = /\b(viewIn|panelIn|turnIn|rescanTravel)\b/u.exec(value)?.[1] ?? null;
  return { name, ms: name === null ? null : firstMs(value) };
}

/** `viewIn` is the opacity-only entry; the other two move or re-track the element. */
function expectClampedEntry(entry: Entry, where: string): void {
  expect(entry.name, `${where} at reduced`).toBe('viewIn');
  expect(entry.ms, `${where} at reduced`).not.toBeNull();
  expect(entry.ms ?? Infinity, `${where} at reduced`).toBeLessThanOrEqual(REDUCED_CLAMP_MS);
}

const pid = (n: number): ProjectId => n as ProjectId;
const basis: RevealBasis = { projectsCovered: 3, projectsTotal: 3, historyComplete: true };
const REVEAL: Reveal = {
  spanDays: { value: 400, basis },
  projectCount: { value: 3, basis },
  languageCount: { value: 2, basis },
  bestYear: { value: 2021, basis },
  playtimeSeconds: { value: 0, basis },
  oldestStillAlive: { projectId: pid(1), firstCommitAt: 1_600_000_000, basis },
};
const REVEAL_DEPS: RevealDeps = {
  nowSecs: 1_700_000_000,
  languageTally: [{ name: 'Rust', count: 2 }],
  referenceCount: 0,
  project: () => ({ name: 'shaped', birthYear: 2020, primaryLanguage: 'Rust' }),
};

function scanView(tier: ResolvedTier): Element {
  render(
    <ScanScreen
      deps={{ jewelFor: () => null, onSkipAhead: () => undefined, onOpenScanSummary: noop }}
      feed={INITIAL_SCAN_FEED}
      rootLine="a root"
      milestone={null}
      tier={tier}
    />,
  );
  const view = document.querySelector('.cdt-fr-view--scan');
  if (view === null) throw new Error('the scan screen mounted no .cdt-fr-view--scan');
  return view;
}

function turnView(tier: ResolvedTier): Element {
  render(
    <TurnScreen
      counts={{ unpushed: 1, dirty: 0, interrupted: 0, total: 3 }}
      worktreeObservedAt={1_700_000_000}
      tier={tier}
      onShowMe={() => undefined}
      onNotNow={() => undefined}
    />,
  );
  const view = document.querySelector('.cdt-fr-view--turn');
  if (view === null) throw new Error('the turn screen mounted no .cdt-fr-view--turn');
  return view;
}

/** The reveal's root is the plain `.cdt-fr-view` entry; its evidence opens on SHOW WORKING. */
function revealWithEvidence(tier: ResolvedTier): { view: Element; evidence: Element } {
  render(<RevealScreen reveal={REVEAL} deps={REVEAL_DEPS} tier={tier} onGoOn={() => undefined} />);
  const panel = screen.getAllByTestId('fr-panel')[0];
  if (panel === undefined) throw new Error('the reveal mounted no panel');
  fireEvent.click(panel);
  const view = document.querySelector('.cdt-fr-view');
  const evidence = document.querySelector('.cdt-fr-evidence');
  if (view === null || evidence === null) throw new Error('no reveal view or evidence mounted');
  return { view, evidence };
}

describe('§11.6 the first run entries, clamped', () => {
  it('each view entry runs viewIn inside the clamp at reduced and nothing at off', () => {
    const surfaces = [
      ['.cdt-fr-view--scan', scanView],
      ['.cdt-fr-view--turn', turnView],
      ['.cdt-fr-view', (tier: ResolvedTier) => revealWithEvidence(tier).view],
    ] as const;
    let checked = 0;
    for (const [where, mount] of surfaces) {
      atTier('full');
      const control = entryOf(mount('full'));
      expect(control.name, `${where} at full`).toBe('viewIn');
      expect(control.ms ?? 0, `${where} at full runs past the clamp`).toBeGreaterThan(
        REDUCED_CLAMP_MS,
      );

      atTier('reduced');
      expectClampedEntry(entryOf(mount('reduced')), where);

      atTier('off');
      expect(entryOf(mount('off')).name, `${where} at off`).toBeNull();
      checked += 1;
    }
    console.warn(`§11.6 first-run views resolved at three tiers: ${String(checked)}`);
    expect(checked).toBe(surfaces.length);
  });

  it('the evidence block drops panelIn for viewIn inside the clamp at reduced, and nothing at off', () => {
    atTier('full');
    const control = entryOf(revealWithEvidence('full').evidence);
    expect(control.name, 'the evidence enters on panelIn at full').toBe('panelIn');
    expect(control.ms ?? 0).toBeGreaterThan(REDUCED_CLAMP_MS);

    atTier('reduced');
    expectClampedEntry(entryOf(revealWithEvidence('reduced').evidence), '.cdt-fr-evidence');

    atTier('off');
    expect(entryOf(revealWithEvidence('off').evidence).name).toBeNull();
  });
});

describe('§11.6 the sigil, clamped', () => {
  /**
   * No product surface passes a glyph yet, so the plate is mounted with one directly inside a
   * `.cdt-card`, which is where `Card` nests it — the hover rule that scales it keys on that
   * ancestor, and it is the rule a bare tier selector would lose to.
   */
  function sigilAt(tier: ResolvedTier): CSSStyleDeclaration {
    atTier(tier);
    render(
      <div className="cdt-card" data-hovered="true">
        <CardPlate surface="card" isArchived={false} art={null} sigil={<span />}>
          {null}
        </CardPlate>
      </div>,
    );
    const sigil = document.querySelector('.cdt-sigil');
    if (sigil === null) throw new Error('no .cdt-sigil mounted');
    return getComputedStyle(sigil);
  }

  it('scales on hover at full, and at reduced and off takes no transform', () => {
    expect(sigilAt('full').transform, 'the control: a hovered sigil scales at full').not.toBe(
      'none',
    );
    for (const tier of ['reduced', 'off'] as const) {
      expect(sigilAt(tier).transform, tier).toBe('none');
    }
  });

  it('transitions colour inside the clamp at reduced, and nothing at off', () => {
    const full = sigilAt('full').transition;
    expect(full, 'the control: the transform transitions at full').toMatch(/\btransform\b/u);

    const reduced = sigilAt('reduced').transition;
    expect(reduced).not.toMatch(/\btransform\b/u);
    expect(reduced).toMatch(/\bcolor\b/u);
    expect(firstMs(reduced) ?? Infinity).toBeLessThanOrEqual(REDUCED_CLAMP_MS);

    const off = sigilAt('off').transition;
    expect(firstMs(off) ?? 0, `off resolved a timed transition: ${off}`).toBe(0);
  });
});

describe('§11.6 the rescan line travels at full only', () => {
  it('the travelling band still runs at full, and is gone at reduced and off', () => {
    const props = {
      trigger: 'launch',
      running: true,
      elapsedMs: 600,
      onOpenSummary: () => undefined,
    } as const;
    const resolved: Record<string, string | null> = {};
    for (const tier of TIERS) {
      atTier(tier);
      render(<RescanLine {...props} tier={tier} />);
      const line = document.querySelector('.cdt-fr-rescan-line');
      if (line === null) throw new Error(`no rescan line mounted at ${tier}`);
      resolved[tier] = entryOf(line).name;
    }
    // The CSS scope added beside the code gate must not have stopped the band where it belongs.
    expect(resolved['full']).toBe('rescanTravel');
    expect(resolved['reduced']).toBeNull();
    expect(resolved['off']).toBeNull();
  });
});

/**
 * The escapes the checker's specificity pass named, **where jsdom can tell a fix from none**.
 *
 * Three it cannot, stated rather than asserted, because a test that passes on the broken sheet is
 * not a guard — `scripts/check-motion-clamp.mjs` is:
 *  - **the hovered condition dot and data strip.** jsdom weighs a rule by the heaviest selector in
 *    its comma list, and the old `transform: none` rule listed `.cdt-card:active` at (0,3,0), so
 *    jsdom resolved `none` over the (0,3,0) hover rule on the broken sheet too. A browser weighs
 *    the (0,2,0) selector that matched, and the hover rule won;
 *  - **SHOW ME's hover lift.** jsdom matches no `:hover`, so neither state can be resolved;
 *  - **`off` naming the dot, the strip and the layers.** `[data-effects-tier='off'] .cdt-card *`
 *    already reached each inside a card, so the sheet resolved the same before and after; the
 *    checker reads the element a rule styles, and a universal descendant names none.
 */
describe('§11.6 the merging tile, clamped', () => {
  function mergingTile(tier: ResolvedTier): CSSStyleDeclaration {
    atTier(tier);
    const events: ScanFeedEvent[] = [
      { kind: 'upserted', id: 1 as ProjectId, name: 'a', primaryLanguage: 'Rust' },
      { kind: 'upserted', id: 2 as ProjectId, name: 'b', primaryLanguage: 'Rust' },
      { kind: 'flush' },
      { kind: 'merged', from: 1 as ProjectId, into: 2 as ProjectId },
    ];
    const feed = events.reduce(scanFeedReducer, INITIAL_SCAN_FEED);
    render(
      <ScanScreen
        deps={{ jewelFor: () => null, onSkipAhead: () => undefined, onOpenScanSummary: noop }}
        feed={feed}
        rootLine="a root"
        milestone={null}
        tier={tier}
      />,
    );
    const tile = document.querySelector('.cdt-fr-tile[data-merged="true"]');
    if (tile === null) throw new Error('no merging tile mounted');
    return getComputedStyle(tile);
  }

  it('fades on opacity inside the clamp at reduced and not at all at off', () => {
    const full = mergingTile('full').transition;
    expect(full, 'the control: the fade runs past the clamp at full').toMatch(/\bopacity\b/u);
    expect(firstMs(full) ?? 0).toBeGreaterThan(REDUCED_CLAMP_MS);

    const reduced = mergingTile('reduced').transition;
    expect(reduced).toMatch(/\bopacity\b/u);
    expect(firstMs(reduced) ?? Infinity).toBeLessThanOrEqual(REDUCED_CLAMP_MS);

    const off = mergingTile('off').transition;
    expect(firstMs(off) ?? 0, `off resolved a timed transition: ${off}`).toBe(0);
  });
});

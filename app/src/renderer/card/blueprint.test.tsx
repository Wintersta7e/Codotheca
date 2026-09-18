import { act, cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ReactElement, ReactNode } from 'react';
import type { InstallPreview, ProjectRow, Rendition, SceneHash } from '../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../project/deps';
import { renditionFor } from '../art/useCardBitmap';
import { INSTALL_LABEL } from '../install/InstallControl';
import cardCss from '../styles/card.css?raw';
import motionCss from '../styles/motion.css?raw';
import { notClonedRow } from '../testing/projectRow';
import { withProjectDeps } from '../testing/deps';
import { LADDER_RUNGS, TOKENS } from '../theme/tokens';
import { ProjectCard, type ProjectCardProps } from './ProjectCard';
import { frameToken, paintsLadderRung } from './completion';

const HASH = '0123456789abcdef'.repeat(4) as SceneHash;

/** Every address a mounted card asked a detached `Image` to decode, in order. */
let requested: string[] = [];
/**
 * The addresses a raster actually exists at. **The 404 is the whole point**: the shell answers a
 * missing rendition with 404 (`artProtocol.ts`), the decode rejects, and §7.5's plate stands.
 *
 * An `Image` stub that resolves whatever it is handed cannot tell a card that *composed* an
 * address from a card that got one from a core that had rendered it — which is exactly how the
 * grid tile shipped asking for a file nothing writes, with a green suite.
 */
let rendered = new Set<string>();

class ArtServer {
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  #src = '';
  get src(): string {
    return this.#src;
  }
  set src(value: string) {
    this.#src = value;
    requested.push(value);
    queueMicrotask(() => {
      if (rendered.has(value)) this.onload?.();
      else this.onerror?.();
    });
  }
}

/**
 * The core's half. §23.5: **the request is the demand** — `art.url` for a blueprint rendition is
 * what causes `blueprint_address` to rasterise the file, so this fake renders on being asked and
 * returns the address it rendered. Nothing else writes a blueprint: J5 draws the `card` rendition
 * only, and the shell serves files rather than making them.
 */
function artServer(): {
  readonly request: ReturnType<typeof vi.fn>;
  readonly wrapper: (props: { children: ReactNode }) => ReactElement;
} {
  const request = vi.fn((name: string, args: { hash: string; rendition: Rendition }) => {
    if (name !== 'art.url') return Promise.resolve(undefined);
    const address = `codotheca://art/${args.hash}/${args.rendition}`;
    rendered.add(address);
    return Promise.resolve(address);
  });
  const deps = {
    request,
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
    installStart: () =>
      Promise.resolve({ kind: 'started' as const, start: { runId: 1, refusedBecause: null } }),
    installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
    pickRoot: () => Promise.resolve({ kind: 'cancelled' as const }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => 1_800_000_000,
  } as unknown as ProjectPageDeps;
  return {
    request,
    wrapper: ({ children }) => (
      <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>
    ),
  };
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  requested = [];
  rendered = new Set<string>();
  document.head.querySelectorAll('style').forEach((node) => {
    node.remove();
  });
  document.documentElement.removeAttribute('data-effects-tier');
});

const row = (over: Partial<ProjectRow> = {}): ProjectRow =>
  notClonedRow({ artSceneHash: HASH, artState: 'ready', ...over });

const props = (over: Partial<ProjectCardProps> = {}): ProjectCardProps => ({
  row: row(),
  density: 186,
  rendition: 'card',
  selected: false,
  focused: false,
  now: 1_800_000_000,
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

const frameOf = (container: HTMLElement): string =>
  container.querySelector<HTMLElement>('.cdt-card-frame')?.style.getPropertyValue('--cdt-frame') ??
  '';

describe('§23.5: the blueprint frame is --tier-blue, and it is not a rung', () => {
  it('resolves var(--tier-blue) on the real frame element', () => {
    expect(frameOf(draw())).toBe('var(--tier-blue)');
  });

  it('bites: a located row is unchanged and still takes the unknown frame', () => {
    const located = notClonedRow({
      artSceneHash: HASH,
      primaryLocation: { id: 10 as never, pathDisplay: '/w/row' },
      presence: 'present',
    });
    expect(frameOf(draw({ row: located }))).toBe('var(--unknown)');
  });

  it('is decided above the ladder, so it is not a member of it', () => {
    expect(paintsLadderRung(TOKENS['tier-blue'])).toBe(false);
    expect(paintsLadderRung('#2f4a5c')).toBe(false);
    // The counter-case, so the assertion is not vacuous: every rung IS one.
    expect(LADDER_RUNGS.length).toBeGreaterThan(0);
    for (const rung of LADDER_RUNGS) expect(paintsLadderRung(rung)).toBe(true);
  });

  it('draws no member of the completion ladder anywhere in the tile', () => {
    // The tile writes token **references** — `var(--tier-blue)` — and `paintsLadderRung` compares
    // hex, so a scan over the raw inline values could never fail. Every reference is resolved
    // through TOKENS first, and the resolved count is asserted: a run that resolved nothing is a
    // run that scanned nothing.
    const container = draw();
    const raw = [...container.querySelectorAll<HTMLElement>('*')].flatMap((el) => [
      el.style.getPropertyValue('--cdt-frame'),
      el.style.backgroundColor,
      el.style.color,
      el.style.borderColor,
    ]);
    const resolved: string[] = [];
    for (const value of raw) {
      const ref = /^var\(--([a-z0-9-]+)\)$/u.exec(value.trim());
      if (ref !== null) {
        const token = TOKENS[ref[1] as keyof typeof TOKENS] as string | undefined;
        if (token !== undefined) resolved.push(token);
        continue;
      }
      if (value !== '') resolved.push(value);
    }
    expect(resolved.length, 'the scan resolved no painted value').toBeGreaterThan(0);
    for (const value of resolved) {
      expect(paintsLadderRung(value), `${value} is a completion rung`).toBe(false);
    }
  });

  it('keeps §7.7a‘s uncomputed rank treatment, because completion is still NULL', () => {
    const container = draw();
    expect(container.querySelector('.cdt-rank-glyph')?.textContent ?? '').toContain('—');
    expect(container.querySelector('.cdt-rank-label')?.textContent).toBe('NOT COMPUTED');
    expect(container.querySelector('.cdt-frame-gap')).not.toBeNull();
  });

  it('reference still wins the frame, and the pair is what decides it', () => {
    expect(frameToken({ isReference: false, hasWorkingCopy: true })).toBe('unknown');
    expect(frameToken({ isReference: false, hasWorkingCopy: false })).toBe('tier-blue');
    expect(frameToken({ isReference: true, hasWorkingCopy: true })).toBe('tier-ref');
    expect(frameToken({ isReference: true, hasWorkingCopy: false })).toBe('tier-ref');
  });
});

describe('§23.5: the tile asks for the pass it needs, at its own address', () => {
  it('maps each surface to its own blueprint name', () => {
    expect(renditionFor('card', true)).toBe('card');
    expect(renditionFor('hero', true)).toBe('hero');
    expect(renditionFor('card', false)).toBe('card-blueprint');
    expect(renditionFor('hero', false)).toBe('hero-blueprint');
  });

  /**
   * The bar this replaces asserted the **request** and passed against a card that made none: an
   * `Image` stub that resolves anything cannot tell a composed address from an answered one. What
   * is asserted here is a **bitmap on screen**, over a fake that renders only what it was asked
   * for — which is the production path, because a blueprint has no writer but the request itself.
   */
  it('paints a decoded blueprint on a not-cloned tile', async () => {
    vi.stubGlobal('Image', ArtServer);
    const { request, wrapper } = artServer();
    let container!: HTMLElement;
    await act(async () => {
      container = render(<ProjectCard {...props()} />, { wrapper }).container;
      await Promise.resolve();
      await Promise.resolve();
    });
    // The demand, and the pass it demanded.
    expect(request).toHaveBeenCalledWith('art.url', {
      hash: HASH,
      rendition: 'card-blueprint',
    });
    // And the bitmap that demand produced, decoded and mounted.
    const img = container.querySelector<HTMLImageElement>('img.cdt-art');
    expect(img, 'the tile painted no bitmap — the plate is standing').not.toBeNull();
    expect(img?.getAttribute('src')).toBe(`codotheca://art/${HASH}/card-blueprint`);
    expect(requested).toContain(`codotheca://art/${HASH}/card-blueprint`);
    expect(requested).not.toContain(`codotheca://art/${HASH}/card`);
  });

  it('paints nothing when the core answers no address, rather than a broken image', async () => {
    // §7.5: an unrendered rendition is a 404 and the plate is the finished fallback.
    vi.stubGlobal('Image', ArtServer);
    const request = vi.fn(() => Promise.resolve(''));
    const deps = {
      request,
      relocate: () => Promise.resolve({ kind: 'cancelled' }),
      uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
      installStart: () =>
        Promise.resolve({ kind: 'started' as const, start: { runId: 1, refusedBecause: null } }),
      installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
      pickRoot: () => Promise.resolve({ kind: 'cancelled' as const }),
      openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
      subscribe: () => () => undefined,
      now: () => 1_800_000_000,
    } as unknown as ProjectPageDeps;
    let container!: HTMLElement;
    await act(async () => {
      container = render(<ProjectCard {...props()} />, {
        wrapper: ({ children }) => (
          <ProjectPageDepsContext.Provider value={deps}>{children}</ProjectPageDepsContext.Provider>
        ),
      }).container;
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(request).toHaveBeenCalled();
    expect(container.querySelector('img.cdt-art')).toBeNull();
  });

  it('asks for nothing on a located tile, and paints the card J5 already wrote', async () => {
    vi.stubGlobal('Image', ArtServer);
    // J5 wrote this one during the scan, so it exists without anybody asking.
    rendered.add(`codotheca://art/${HASH}/card`);
    const { request, wrapper } = artServer();
    const located = notClonedRow({
      artSceneHash: HASH,
      artState: 'ready',
      primaryLocation: { id: 10 as never, pathDisplay: '/w/row' },
      presence: 'present',
    });
    let container!: HTMLElement;
    await act(async () => {
      container = render(<ProjectCard {...props({ row: located })} />, { wrapper }).container;
      await Promise.resolve();
      await Promise.resolve();
    });
    // Byte-identical to before this change: a located tile issues no command at all.
    expect(request).not.toHaveBeenCalled();
    expect(container.querySelector<HTMLImageElement>('img.cdt-art')?.getAttribute('src')).toBe(
      `codotheca://art/${HASH}/card`,
    );
    expect(requested).not.toContain(`codotheca://art/${HASH}/card-blueprint`);
  });
});

/**
 * *"Only genuinely absent things (blueprint, reference) sit at zero."* `flicker.ts` already
 * returns false under a null `presence`, so this needs **no new rule** — it needs an assertion,
 * at every effects tier, resolved on a real element rather than matched in the stylesheet.
 */
describe('§11.6: the blueprint tile produces no dip at any tier', () => {
  const haloOpacityAt = (tier: 'full' | 'reduced' | 'off', haloOpacity: number): string => {
    expect(cardCss.length, 'card.css imported as an empty string').toBeGreaterThan(0);
    expect(motionCss.length, 'motion.css imported as an empty string').toBeGreaterThan(0);
    const style = document.createElement('style');
    style.textContent = `${cardCss}\n${motionCss}`;
    document.head.append(style);
    document.documentElement.setAttribute('data-effects-tier', tier);
    const container = draw({ haloOpacity });
    const halo = container.querySelector('.cdt-card-halo');
    if (halo === null) throw new Error('the card mounted no .cdt-card-halo');
    return getComputedStyle(halo).opacity;
  };

  it('is clamped at reduced and at off, on an element the clamp really selects', () => {
    expect(haloOpacityAt('full', 0.8)).toContain('--cdt-halo-opacity');
    cleanup();
    expect(haloOpacityAt('reduced', 0.8)).toBe('1');
    cleanup();
    expect(haloOpacityAt('off', 0.8)).toBe('1');
  });

  it('is never a candidate, so the driver never picks it in the first place', async () => {
    const { flickerEligible } = await import('../motion/flicker');
    for (const signal of ['neglected', 'abandoned'] as const) {
      expect(flickerEligible(row({ conditionSignal: signal }))).toBe(false);
    }
    // The counter-case: the same band on a present copy is eligible, so the rule above is the
    // location and not the band.
    expect(
      flickerEligible({ conditionSignal: 'neglected', isReference: false, presence: 'present' }),
    ).toBe(true);
  });
});

/**
 * R36's shape a second time. The clamp named `.cdt-plate`, which carries §7.3a's static greebling,
 * while the 700 ms sweep actually lives on `.cdt-specular` (`card.css:176-190`) — so `reduced`
 * stripped a per-card texture every tier keeps AND left the sweep running, and `off` stopped the
 * sweep only through the blanket `.cdt-card *` rule. Nothing could observe either direction while
 * the document element still read `auto`.
 *
 * Resolved on real elements at every tier, never matched in the stylesheet.
 */
describe('§7.8: the specular sweep goes below full and the greebling never does', () => {
  const mountAt = (tier: 'full' | 'reduced' | 'off'): HTMLElement => {
    expect(cardCss.length, 'card.css imported as an empty string').toBeGreaterThan(0);
    expect(motionCss.length, 'motion.css imported as an empty string').toBeGreaterThan(0);
    const style = document.createElement('style');
    style.textContent = `${cardCss}\n${motionCss}`;
    document.head.append(style);
    document.documentElement.setAttribute('data-effects-tier', tier);
    return draw();
  };

  const resolve = (container: HTMLElement, selector: string): CSSStyleDeclaration => {
    const element = container.querySelector(selector);
    if (element === null) throw new Error(`the card mounted no ${selector}`);
    return getComputedStyle(element);
  };

  it('hides the sweep below full, on the element that really carries it', () => {
    expect(resolve(mountAt('full'), '.cdt-specular').display).not.toBe('none');
    cleanup();
    expect(resolve(mountAt('reduced'), '.cdt-specular').display).toBe('none');
    cleanup();
    expect(resolve(mountAt('off'), '.cdt-specular').display).toBe('none');
  });

  it('keeps the plate greebling at every tier, because a texture is not a highlight', () => {
    for (const tier of ['full', 'reduced', 'off'] as const) {
      const background = resolve(mountAt(tier), '.cdt-plate').backgroundImage;
      expect(background, `greebling dropped at ${tier}`).toContain('--cdt-greebling');
      cleanup();
    }
  });
});

/**
 * [p2] §24.3d's card slot. **The request is the demand** (§7.6), which on a virtualized grid is
 * what keeps a library of hundreds of not-cloned projects from previewing all of them at once:
 * a tile is mounted only while it is near the viewport, so it is the tile that asks.
 */
describe('§24.3d: the blueprint tile demands its own install preview', () => {
  it('asks once, naming itself, when it has no working copy', () => {
    const onNeed = vi.fn();
    draw({ onNeedInstallPreview: onNeed });
    expect(onNeed).toHaveBeenCalledTimes(1);
    expect(onNeed).toHaveBeenCalledWith(row().id);
  });

  it('asks for nothing on a tile that already has a copy to play', () => {
    const onNeed = vi.fn();
    draw({
      row: { ...row(), primaryLocation: { id: 4, pathDisplay: '~/work/aurora' } } as ProjectRow,
      onNeedInstallPreview: onNeed,
    });
    expect(onNeed).not.toHaveBeenCalled();
  });

  it('offers nothing while no preview has arrived, rather than an empty control', () => {
    const container = draw({ onNeedInstallPreview: vi.fn() });
    expect(container.querySelector('.cdt-card-install')).toBeNull();
  });

  it('mounts the control once the core answers', () => {
    const container = draw({
      onNeedInstallPreview: vi.fn(),
      installPreview: {
        destination: { display: '~/work/aurora' },
        refusedBecause: null,
      } as unknown as InstallPreview,
    });
    const slot = container.querySelector('.cdt-card-install');
    expect(slot, 'the blueprint tile offered no install').not.toBeNull();
    expect(slot?.textContent).toContain(INSTALL_LABEL);
  });
});

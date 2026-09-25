import { act, cleanup, render, renderHook, screen } from '@testing-library/react';
import type { ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SceneHash } from '../../generated/protocol';
import {
  type CardBitmapInput,
  artUrl,
  useCardBitmap,
  useInstalledRenditionFlip,
} from './useCardBitmap';
import { noop } from '../noop';

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const hash = (s: string): SceneHash => s as SceneHash;

function Probe(props: CardBitmapInput): ReactElement {
  const bitmap = useCardBitmap(props);
  return (
    <span data-testid="src" data-held={String(bitmap.held)}>
      {bitmap.src ?? 'plate'}
    </span>
  );
}

const shown = (): string => screen.getByTestId('src').textContent ?? '';
const held = (): string | null => screen.getByTestId('src').getAttribute('data-held');

/** Runs `fn` and flushes the decode's microtask inside one `act`, so React sees both. */
const settle = async (fn: () => void = noop): Promise<void> => {
  await act(async () => {
    fn();
    await Promise.resolve();
  });
};

const instant = (): Promise<void> => Promise.resolve();

describe('the address is two segments and is built in one place', () => {
  it('composes scheme, host, hash and rendition', () => {
    expect(artUrl(hash('ab12'), 'card')).toBe('codotheca://art/ab12/card');
    expect(artUrl(hash('ab12'), 'hero')).toBe('codotheca://art/ab12/hero');
  });

  it('has no address for a project whose scene has never been generated', () => {
    expect(artUrl(null, 'card')).toBeNull();
    expect(artUrl(hash(''), 'card')).toBeNull();
  });

  // §7.6's address is two-segment. A rendition folded into the filename would make
  // `codotheca://art/<hash>` a fetchable address, which criterion 62 asserts it is not.
  it('carries the rendition as its own segment, so the two renditions differ', () => {
    expect(artUrl(hash('ab12'), 'card')).not.toBe(artUrl(hash('ab12'), 'hero'));
    expect(artUrl(hash('ab12'), 'card')?.split('/')).toHaveLength(5);
  });
});

describe('§7.1a: the plate is held until that exact scene_hash has decoded', () => {
  it('shows the plate — no src at all — until the decode resolves', async () => {
    let release = noop;
    const decode = async (): Promise<void> => {
      await new Promise<void>((resolve) => {
        release = resolve;
      });
    };
    render(<Probe sceneHash={hash('aa')} rendition="card" artState="ready" decode={decode} />);
    expect(shown()).toBe('plate');
    await settle(() => {
      release();
    });
    expect(shown()).toBe('codotheca://art/aa/card');
  });

  it('holds the OLD bitmap while a new hash decodes, so no card changes under the pointer', async () => {
    let release = noop;
    const stalled = async (): Promise<void> => {
      await new Promise<void>((resolve) => {
        release = resolve;
      });
    };
    const view = render(
      <Probe sceneHash={hash('aa')} rendition="card" artState="ready" decode={instant} />,
    );
    await settle();
    expect(shown()).toBe('codotheca://art/aa/card');
    expect(held()).toBe('false');

    view.rerender(
      <Probe sceneHash={hash('bb')} rendition="card" artState="ready" decode={stalled} />,
    );
    // The held state is the requirement, not the destination: this is the frame a user would
    // have caught the card changing in, and it must show the old bitmap and say it is holding.
    expect(shown()).toBe('codotheca://art/aa/card');
    expect(held()).toBe('true');

    await settle(() => {
      release();
    });
    expect(shown()).toBe('codotheca://art/bb/card');
    expect(held()).toBe('false');
  });

  it('never swaps to a decode that resolved after the address moved on', async () => {
    let releaseFirst = noop;
    const stalled = async (): Promise<void> => {
      await new Promise<void>((resolve) => {
        releaseFirst = resolve;
      });
    };
    const view = render(
      <Probe sceneHash={hash('aa')} rendition="card" artState="ready" decode={stalled} />,
    );
    view.rerender(
      <Probe sceneHash={hash('bb')} rendition="card" artState="ready" decode={instant} />,
    );
    await settle(() => {
      releaseFirst();
    });
    expect(shown()).toBe('codotheca://art/bb/card');
  });

  it('falls back to the plate when the file is gone, and never renders a hole', async () => {
    const fails = (): Promise<void> => Promise.reject(new Error('404'));
    render(<Probe sceneHash={hash('cc')} rendition="card" artState="ready" decode={fails} />);
    await settle();
    expect(shown()).toBe('plate');
  });

  it('does not address a failed row at all — §7.5 makes the nameplate a finished card', () => {
    render(<Probe sceneHash={hash('dd')} rendition="card" artState="failed" />);
    expect(shown()).toBe('plate');
  });
});

describe('the production decode is wired, not only the injected one', () => {
  // The seam exists so a test can hold a decode open. A seam whose only implementation is the
  // fake is the defect four rulings here were written against, so this exercises the default.
  it('decodes a detached Image at the composed address when no decode is injected', async () => {
    const seen: string[] = [];
    class FakeImage {
      public onload: (() => void) | null = null;
      public onerror: (() => void) | null = null;
      private value = '';
      public get src(): string {
        return this.value;
      }
      public set src(next: string) {
        this.value = next;
        seen.push(next);
        this.onload?.();
      }
    }
    vi.stubGlobal('Image', FakeImage);

    render(<Probe sceneHash={hash('ff')} rendition="card" artState="ready" />);
    await settle();
    expect(seen).toEqual(['codotheca://art/ff/card']);
    expect(shown()).toBe('codotheca://art/ff/card');
  });

  it('keeps the plate when the detached Image errors', async () => {
    class FailingImage {
      public onload: (() => void) | null = null;
      public onerror: (() => void) | null = null;
      public set src(_next: string) {
        this.onerror?.();
      }
    }
    vi.stubGlobal('Image', FailingImage);

    render(<Probe sceneHash={hash('gg')} rendition="card" artState="ready" />);
    await settle();
    expect(shown()).toBe('plate');
  });
});

describe('the hero takes its address from art.url, because that request is the demand', () => {
  it('uses the caller‘s src and treats the empty answer as keep the plate', async () => {
    render(
      <Probe sceneHash={hash('ee')} rendition="hero" artState="ready" src="" decode={instant} />,
    );
    await settle();
    expect(shown()).toBe('plate');
  });

  it('prefers the caller‘s answer over the address it could have composed', async () => {
    render(
      <Probe
        sceneHash={hash('ee')}
        rendition="hero"
        artState="ready"
        src="codotheca://art/other/hero"
        decode={instant}
      />,
    );
    await settle();
    expect(shown()).toBe('codotheca://art/other/hero');
  });
});

// ---------------------------------------------------------------------------
// [p2] §24.4's rendition swap — AC-P2-24-11.
// ---------------------------------------------------------------------------

describe('the install-time rendition flip', () => {
  /** A decodable image: `onload` fires on the next tick, as a cached raster would. */
  class LoadingImage {
    onload: (() => void) | null = null;
    onerror: (() => void) | null = null;
    set src(_value: string) {
      queueMicrotask(() => this.onload?.());
    }
  }

  /** One that never decodes, so the blueprint must stay up. */
  class NeverLoadingImage {
    onload: (() => void) | null = null;
    onerror: (() => void) | null = null;
    set src(_value: string) {
      /* never resolves */
    }
  }

  const realImage = globalThis.Image;
  afterEach(() => {
    globalThis.Image = realImage;
  });

  /** p2-23's pieces, asserted present before anything below is meaningful. */
  it('rests on four rendition variants whose blueprint slugs address differently', () => {
    expect(artUrl('abc' as SceneHash, 'card-blueprint')).not.toBe(
      artUrl('abc' as SceneHash, 'hero-blueprint'),
    );
    expect(artUrl('abc' as SceneHash, 'card-blueprint')).toContain('/card-blueprint');
    expect(artUrl('abc' as SceneHash, 'hero-blueprint')).toContain('/hero-blueprint');
  });

  it('flips the tile card-blueprint to card and the hero hero-blueprint to hero', async () => {
    globalThis.Image = LoadingImage as unknown as typeof Image;

    const tile = renderHook(
      ({ done }: { done: boolean }) =>
        useInstalledRenditionFlip('card', 'abc' as SceneHash, false, done),
      { initialProps: { done: false } },
    );
    const hero = renderHook(
      ({ done }: { done: boolean }) =>
        useInstalledRenditionFlip('hero', 'abc' as SceneHash, false, done),
      { initialProps: { done: false } },
    );
    expect(tile.result.current).toBe('card-blueprint');
    expect(hero.result.current).toBe('hero-blueprint');

    tile.rerender({ done: true });
    hero.rerender({ done: true });
    await act(() => Promise.resolve());

    expect(tile.result.current).toBe('card');
    expect(hero.result.current).toBe('hero');
    // R47's whole point: the two never cross. A card raster served for a hero is the failure a
    // single `blueprint` variant would have made unavoidable.
    expect(tile.result.current).not.toBe('hero');
    expect(hero.result.current).not.toBe('card');
  });

  it('holds the decoded blueprint until the new rendition has decoded', async () => {
    globalThis.Image = NeverLoadingImage as unknown as typeof Image;
    const tile = renderHook(
      ({ done }: { done: boolean }) =>
        useInstalledRenditionFlip('card', 'abc' as SceneHash, false, done),
      { initialProps: { done: false } },
    );
    tile.rerender({ done: true });
    await act(() => Promise.resolve());
    expect(tile.result.current).toBe('card-blueprint');
  });

  it('holds the blueprint when there is no scene to address at all', async () => {
    globalThis.Image = LoadingImage as unknown as typeof Image;
    const tile = renderHook(
      ({ done }: { done: boolean }) => useInstalledRenditionFlip('card', null, false, done),
      { initialProps: { done: false } },
    );
    tile.rerender({ done: true });
    await act(() => Promise.resolve());
    expect(tile.result.current).toBe('card-blueprint');
  });
});

import { act, cleanup, render, screen } from '@testing-library/react';
import type { ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SceneHash } from '../../generated/protocol';
import { type CardBitmapInput, artUrl, useCardBitmap } from './useCardBitmap';

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
const settle = async (fn: () => void = (): void => {}): Promise<void> => {
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
    let release = (): void => {};
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
    let release = (): void => {};
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
    let releaseFirst = (): void => {};
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

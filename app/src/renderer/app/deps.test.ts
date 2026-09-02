import { afterEach, describe, expect, it, vi } from 'vitest';

import type { CodothecaBridge } from '../../shared/bridge';
import type { RendererEvent } from '../../shared/channels';
import type { ProjectPageDeps } from '../project/deps';
import { createDefaultAppDeps, type AppDeps } from './deps';

type EventBatchListener = (batch: unknown) => void;

interface StubBridge {
  readonly bridge: CodothecaBridge;
  /** Every registration the bridge received on the disposer-less events channel. */
  readonly registrations: EventBatchListener[];
}

function stubBridge(): StubBridge {
  const registrations: EventBatchListener[] = [];
  const bridge: CodothecaBridge = {
    protocolVersion: 1,
    effectsTier: 'full',
    effectsTierSource: 'boot-file',
    paintFailForcedAt: null,
    request: () => Promise.resolve({ ok: true, value: null }),
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    pickRoot: () => Promise.resolve({ kind: 'cancelled' }),
    onCoreStatus: () => undefined,
    onCoreEvents: (cb) => {
      registrations.push(cb);
    },
    onOpenPalette: () => undefined,
    pickExecutable: () => Promise.resolve(null),
    reveal: () => Promise.resolve(null),
    indexLocation: () => Promise.resolve(null),
    clearPaintFailure: () => Promise.resolve(null),
    onShortcutState: () => undefined,
  };
  return { bridge, registrations };
}

function install(bridge: CodothecaBridge): void {
  Object.defineProperty(window, 'codotheca', { value: bridge, configurable: true });
}

afterEach(() => {
  Reflect.deleteProperty(window, 'codotheca');
  vi.useRealTimers();
});

describe('AppDeps', () => {
  it('satisfies ProjectPageDeps, so the project page needs no second construction', () => {
    const stub = stubBridge();
    install(stub.bridge);
    const deps: AppDeps = createDefaultAppDeps();

    // Compile-level: an AppDeps *is* a ProjectPageDeps. If this stops holding, the mount would
    // have to build a second deps object, which is the drift this interface exists to prevent.
    const asPageDeps: ProjectPageDeps = deps;

    // And at runtime, because a structural type erases at the boundary: every member the page
    // reads has to be callable on the object the mount actually provides.
    for (const member of ['request', 'relocate', 'subscribe', 'now'] as const) {
      expect(typeof asPageDeps[member]).toBe('function');
    }
  });

  it('fans one events registration out to every subscriber', () => {
    const stub = stubBridge();
    install(stub.bridge);
    const deps = createDefaultAppDeps();

    const first: RendererEvent[] = [];
    const second: RendererEvent[] = [];
    const dropFirst = deps.subscribe((event) => first.push(event));
    deps.subscribe((event) => second.push(event));

    // `onCoreEvents` has no disposer, so a second registration would not replace the first —
    // one of the two subscribers would simply never be called again.
    expect(stub.registrations).toHaveLength(1);

    const emit = stub.registrations[0];
    expect(emit).toBeDefined();
    emit?.([{ topic: 'projects', event: 'upserted', data: { row: null } }]);

    expect(first).toHaveLength(1);
    expect(second).toHaveLength(1);
    expect(second[0]?.event).toBe('upserted');

    dropFirst();
    emit?.([{ topic: 'scan', event: 'progress', data: null }]);
    expect(first).toHaveLength(1);
    expect(second).toHaveLength(2);
  });

  it('reads seconds and milliseconds off one clock', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-03-04T05:06:07.890Z'));
    const stub = stubBridge();
    install(stub.bridge);
    const deps = createDefaultAppDeps();

    // The shelf's `now` and the card's bench timer are the same instant read at two scales. Two
    // clocks drift by up to a second, which is a visible off-by-one in an `as of` clause.
    expect(deps.now()).toBe(Math.floor(deps.nowMs() / 1000));
    expect(deps.nowMs()).toBe(Date.parse('2026-03-04T05:06:07.890Z'));
  });

  it('gives every disposer-less shell channel one registration and many subscribers', () => {
    const openPalette: (() => void)[] = [];
    const stub = stubBridge();
    const bridge: CodothecaBridge = {
      ...stub.bridge,
      onOpenPalette: (cb) => {
        openPalette.push(cb);
      },
    };
    install(bridge);
    const deps = createDefaultAppDeps();

    let a = 0;
    let b = 0;
    const dropA = deps.onOpenPalette(() => {
      a += 1;
    });
    deps.onOpenPalette(() => {
      b += 1;
    });
    expect(openPalette).toHaveLength(1);

    openPalette[0]?.();
    expect(a).toBe(1);
    expect(b).toBe(1);

    dropA();
    openPalette[0]?.();
    expect(a).toBe(1);
    expect(b).toBe(2);
  });
});

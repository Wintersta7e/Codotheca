/**
 * The renderer's whole outside world, injected once.
 *
 * `project/deps.ts` already established this shape for one surface. `AppDeps` is the superset,
 * and it **extends** `ProjectPageDeps` rather than restating it: the project page's context is
 * then provided from the same object, so there is one construction of the door and not two
 * that can disagree about which clock or which fan-out they hold.
 *
 * Every screen below stays renderable with no Electron and no core behind it, which is what
 * makes the mount testable at all.
 */
import { createContext, useContext } from 'react';

import type { PickRootReply, RevealTarget, ShortcutState } from '../../shared/channels';
import type { CoreStatus } from '../../shared/coreStatus';
import type { EffectsTier, EffectsTierSource } from '../../shared/effectsTier';
import { call } from '../core/call';
import { createEventFanout, type ProjectPageDeps } from '../project/deps';

// R12/R16: one fan-out, not two. `createEventFanout` is imported from the module that owns it —
// a copy here would mean two registrations on `onCoreEvents`, which has no disposer, so the
// second would not replace the first and one of them would silently receive nothing.
export { createEventFanout } from '../project/deps';

/**
 * The single-value case of the same problem. `onCoreStatus`, `onOpenPalette` and
 * `onShortcutState` all register for the window's lifetime and hand back no disposer, so a hook
 * that registers per mount accumulates listeners. This registers once and hands every caller a
 * disposer of its own.
 *
 * Not `createEventFanout` with a different type argument: that one additionally unwraps a batch
 * and validates each item. Different shapes, so they are not collapsed (R15).
 */
export function createValueFanout<T>(
  register: (cb: (value: T) => void) => void,
): (handler: (value: T) => void) => () => void {
  const handlers = new Set<(value: T) => void>();
  let registered = false;

  return (handler) => {
    if (!registered) {
      registered = true;
      register((value: T) => {
        for (const fn of [...handlers]) {
          try {
            fn(value);
          } catch {
            // One subscriber's failure may not silence the rest of the window.
          }
        }
      });
    }
    handlers.add(handler);
    return () => {
      handlers.delete(handler);
    };
  };
}

export interface AppDeps extends ProjectPageDeps {
  /**
   * Monotonic-enough milliseconds for the first-run beats. `now()` is this clock in seconds —
   * one reading at two scales, never two clocks a second apart.
   */
  readonly nowMs: () => number;
  /** §2.4: the shell owns the folder dialog. A flag goes out; no path comes back in. */
  readonly pickRoot: (confirmLarge: boolean) => Promise<PickRootReply>;
  readonly pickExecutable: (scope: unknown) => Promise<unknown>;
  /** Two named targets, never a path. */
  readonly reveal: (target: RevealTarget) => Promise<unknown>;
  readonly indexLocation: () => Promise<unknown>;
  readonly clearPaintFailure: () => Promise<unknown>;
  readonly onCoreStatus: (cb: (status: CoreStatus) => void) => () => void;
  readonly onShortcutState: (cb: (state: ShortcutState) => void) => () => void;
  readonly onOpenPalette: (cb: () => void) => () => void;
  readonly effectsTier: EffectsTier;
  readonly effectsTierSource: EffectsTierSource;
  readonly paintFailForcedAt: number | null;
  /** §11.2a names the log on every failure window. Display only; empty means none was passed. */
  readonly logPath: string;
}

export const AppDepsContext = createContext<AppDeps | null>(null);

export function useAppDeps(): AppDeps {
  const deps = useContext(AppDepsContext);
  if (deps === null) {
    throw new Error('AppDepsContext is not provided');
  }
  return deps;
}

export function createDefaultAppDeps(): AppDeps {
  const bridge = window.codotheca;
  const nowMs = (): number => Date.now();

  return {
    // `core/call.ts`'s one un-feature-bound wrapper, imported rather than rebuilt: a second
    // `callWith` over the same bridge is a second place for §2.2's retry decision to be made.
    request: call,
    relocate: (locationId) => bridge.relocate(locationId),
    subscribe: createEventFanout((cb) => {
      bridge.onCoreEvents(cb);
    }),
    now: () => Math.floor(nowMs() / 1000),
    nowMs,
    pickRoot: (confirmLarge) => bridge.pickRoot(confirmLarge),
    pickExecutable: (scope) => bridge.pickExecutable(scope),
    reveal: (target) => bridge.reveal(target),
    indexLocation: () => bridge.indexLocation(),
    clearPaintFailure: () => bridge.clearPaintFailure(),
    onCoreStatus: createValueFanout<CoreStatus>((cb) => {
      bridge.onCoreStatus(cb as (status: unknown) => void);
    }),
    onShortcutState: createValueFanout<ShortcutState>((cb) => {
      bridge.onShortcutState(cb);
    }),
    onOpenPalette: createValueFanout<void>((cb) => {
      bridge.onOpenPalette(() => {
        cb(undefined);
      });
    }),
    effectsTier: bridge.effectsTier,
    effectsTierSource: bridge.effectsTierSource,
    paintFailForcedAt: bridge.paintFailForcedAt,
    logPath: bridge.logPath,
  };
}

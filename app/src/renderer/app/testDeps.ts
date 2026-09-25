import type { CommandArgs, CommandName, CommandResult } from '../../generated/protocol.js';
import type { RendererEvent, ShortcutState } from '../../shared/channels.js';
import type { CoreStatus } from '../../shared/coreStatus.js';
import type { AppDeps } from './deps.js';
import { createValueFanout } from './deps.js';

/**
 * An `AppDeps` with no Electron and no core behind it, so every hook and every screen is
 * drivable from a test. Imported by tests only; nothing in the bundle reaches it.
 */
export interface FakeAppDeps {
  readonly deps: AppDeps;
  /** Push one event at every current subscriber, as the shell's batch would. */
  readonly emit: (event: RendererEvent) => void;
  readonly setCoreStatus: (status: CoreStatus) => void;
  readonly setShortcutState: (state: ShortcutState) => void;
  readonly openPalette: () => void;
  /** Every command the surface under test issued, in order. */
  readonly calls: { name: CommandName; args: unknown }[];
  readonly setNow: (unixSeconds: number) => void;
}

export type FakeReplies = {
  [K in CommandName]?: (args: CommandArgs[K]) => CommandResult[K] | Promise<CommandResult[K]>;
};

export function fakeAppDeps(replies: FakeReplies = {}, over: Partial<AppDeps> = {}): FakeAppDeps {
  const subscribers = new Set<(event: RendererEvent) => void>();
  const calls: { name: CommandName; args: unknown }[] = [];
  let nowSeconds = 1_700_000_000;

  let pushStatus: ((status: CoreStatus) => void) | null = null;
  let pushShortcut: ((state: ShortcutState) => void) | null = null;
  let pushPalette: (() => void) | null = null;

  const deps: AppDeps = {
    request: <K extends CommandName>(name: K, args: CommandArgs[K]) => {
      calls.push({ name, args });
      const reply = replies[name];
      if (reply === undefined) {
        return Promise.reject(new Error(`no fake reply for ${name}`));
      }
      // A refusal has to arrive as a rejected promise, the way `CallFailure` does. A fake that
      // threw synchronously would test a shape the real wrapper never produces.
      try {
        return Promise.resolve(reply(args));
      } catch (error: unknown) {
        return Promise.reject(error instanceof Error ? error : new Error(String(error)));
      }
    },
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: (handler) => {
      subscribers.add(handler);
      return () => {
        subscribers.delete(handler);
      };
    },
    now: () => nowSeconds,
    nowMs: () => nowSeconds * 1000,
    pickRoot: () => Promise.resolve({ kind: 'cancelled' }),
    commitSuggestion: () => Promise.resolve({ kind: 'unknown' as const }),
    installStart: () =>
      Promise.resolve({ kind: 'started' as const, start: { runId: 1, refusedBecause: null } }),
    installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
    // The shell's own channels answer a `BridgeReply` envelope, not a bare value — the same
    // shape `registerShellServices` returns. A fake answering `null` would let a caller that
    // forgot to unwrap pass here and throw in the product.
    pickExecutable: () => Promise.resolve({ ok: true, value: null }),
    reveal: () => Promise.resolve({ ok: true, value: null }),
    indexLocation: () =>
      Promise.resolve({ ok: true, value: { pathDisplay: '<index>', sizeBytes: 0 } }),
    clearPaintFailure: () => Promise.resolve({ ok: true, value: null }),
    onCoreStatus: createValueFanout<CoreStatus>((cb) => {
      pushStatus = cb;
    }),
    onShortcutState: createValueFanout<ShortcutState>((cb) => {
      pushShortcut = cb;
    }),
    onOpenPalette: createValueFanout<undefined>((cb) => {
      pushPalette = () => {
        cb(undefined);
      };
    }),
    effectsTier: 'full',
    effectsTierSource: 'boot-file',
    paintFailForcedAt: null,
    reducedMotionOverride: false,
    logPath: '/tmp/codotheca/logs/codotheca.log',
    ...over,
  };

  return {
    deps,
    calls,
    emit: (event) => {
      for (const handler of [...subscribers]) handler(event);
    },
    setCoreStatus: (status) => pushStatus?.(status),
    setShortcutState: (state) => pushShortcut?.(state),
    openPalette: () => pushPalette?.(),
    setNow: (unixSeconds) => {
      nowSeconds = unixSeconds;
    },
  };
}

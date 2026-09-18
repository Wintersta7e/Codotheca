import { afterEach, describe, expect, it, vi } from 'vitest';
import type { CodothecaBridge } from '../../shared/bridge';
import type { BridgeError } from '../../shared/channels';
import { CommandError, createDefaultDeps, createEventFanout } from './deps';

describe('the event fan-out', () => {
  it('registers with the preload exactly once however many handlers subscribe', () => {
    const register = vi.fn();
    const subscribe = createEventFanout(register);
    expect(register).not.toHaveBeenCalled();
    subscribe(() => undefined);
    subscribe(() => undefined);
    expect(register).toHaveBeenCalledTimes(1);
  });

  it('delivers every event of a batch to every live handler', () => {
    let emit: (batch: unknown) => void = () => undefined;
    const subscribe = createEventFanout((cb) => {
      emit = cb;
    });
    const a: unknown[] = [];
    const b: unknown[] = [];
    subscribe((e) => a.push(e));
    subscribe((e) => b.push(e));
    emit([
      { topic: 'projects', event: 'art_ready', data: { projectId: 1 } },
      { topic: 'projects', event: 'upserted', data: { row: { id: 1 } } },
    ]);
    expect(a).toHaveLength(2);
    expect(b).toHaveLength(2);
  });

  it('stops delivering after the returned disposer runs', () => {
    let emit: (batch: unknown) => void = () => undefined;
    const subscribe = createEventFanout((cb) => {
      emit = cb;
    });
    const seen: unknown[] = [];
    const off = subscribe((e) => seen.push(e));
    off();
    emit([{ topic: 'projects', event: 'upserted', data: {} }]);
    expect(seen).toHaveLength(0);
  });

  it('ignores a batch that is not an array of shaped events', () => {
    let emit: (batch: unknown) => void = () => undefined;
    const subscribe = createEventFanout((cb) => {
      emit = cb;
    });
    const seen: unknown[] = [];
    subscribe((e) => seen.push(e));
    emit('nonsense');
    emit([null, 7, { topic: 'projects' }, { event: 'upserted' }]);
    expect(seen).toHaveLength(0);
  });

  it('one throwing handler does not stop the next one', () => {
    let emit: (batch: unknown) => void = () => undefined;
    const subscribe = createEventFanout((cb) => {
      emit = cb;
    });
    const seen: unknown[] = [];
    subscribe(() => {
      throw new Error('boom');
    });
    subscribe((e) => seen.push(e));
    emit([{ topic: 'projects', event: 'upserted', data: {} }]);
    expect(seen).toHaveLength(1);
  });
});

describe('CommandError', () => {
  const base: BridgeError = {
    code: 'PATH_GONE',
    message: 'stat failed',
    outcome: null,
    retryable: false,
  };

  it('carries the closed error code and the outcome, never a raw core message as prose', () => {
    const err = new CommandError(base);
    expect(err.code).toBe('PATH_GONE');
    // §2.2: `null` is *definitely did not take effect*. There is no `failed` outcome.
    expect(err.outcome).toBeNull();
    expect(err.retryable).toBe(false);
    expect(err).toBeInstanceOf(Error);
    expect(err.name).toBe('CommandError');
  });

  it('keeps a may-have-landed outcome distinct from a refusal', () => {
    const err = new CommandError({
      code: 'CORE_RESTARTED',
      message: 'core restarted',
      outcome: 'unknown',
      retryable: true,
    });
    expect(err.outcome).toBe('unknown');
    expect(err.retryable).toBe(true);
  });
});

/**
 * The production seam, not the fake. A deps object that only exists as a test double is the
 * recorded shape of a trait that never gets its real implementation.
 */
describe('createDefaultDeps', () => {
  afterEach(() => {
    Reflect.deleteProperty(window, 'codotheca');
  });

  function installBridge(over: Partial<CodothecaBridge>): void {
    const base: CodothecaBridge = {
      protocolVersion: 2,
      effectsTier: 'auto',
      logPath: '/tmp/codotheca/logs/codotheca.log',
      request: () => Promise.resolve({ ok: true, value: {} }),
      relocate: () => Promise.resolve({ kind: 'cancelled' }),
      openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
      pickRoot: () => Promise.resolve({ kind: 'cancelled' }),
      commitSuggestion: () => Promise.resolve({ kind: 'unknown' as const }),
      uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
      installStart: () => Promise.resolve({ kind: 'started' as const, start: null }),
      installCancel: () => Promise.resolve({ kind: 'cancelled' as const }),
      onCoreStatus: () => undefined,
      onCoreEvents: () => undefined,
      // Plan 15 added this to the bridge on another branch. The fake must carry the whole
      // interface or it stops being evidence about the shape the product actually has.
      onOpenPalette: () => undefined,
      // §11's shell services, added for the same reason.
      effectsTierSource: 'boot-file',
      paintFailForcedAt: null,
      pickExecutable: () => Promise.resolve({ ok: true, value: null }),
      reveal: () => Promise.resolve({ ok: true, value: null }),
      indexLocation: () => Promise.resolve({ ok: true, value: null }),
      clearPaintFailure: () => Promise.resolve({ ok: true, value: null }),
      onShortcutState: () => undefined,
    };
    const bridge: CodothecaBridge = { ...base, ...over };
    Object.defineProperty(window, 'codotheca', { value: bridge, configurable: true });
  }

  it('unwraps a successful reply to its value', async () => {
    installBridge({ request: () => Promise.resolve({ ok: true, value: { id: 7 } }) });
    const deps = createDefaultDeps();
    await expect(deps.request('projects.requeue', { id: 7 as never })).resolves.toEqual({ id: 7 });
  });

  it('throws a CommandError carrying the three retry fields, never a bare string', async () => {
    installBridge({
      request: () =>
        Promise.resolve({
          ok: false,
          error: { code: 'REPO_UNREADABLE', message: 'nope', outcome: null, retryable: false },
        }),
    });
    const deps = createDefaultDeps();
    await expect(deps.request('projects.requeue', { id: 7 as never })).rejects.toBeInstanceOf(
      CommandError,
    );
  });

  it('reads the clock in unix seconds, matching every timestamp on the wire', () => {
    installBridge({});
    vi.spyOn(Date, 'now').mockReturnValue(1_800_000_123_456);
    expect(createDefaultDeps().now()).toBe(1_800_000_123);
  });

  it('installs exactly one core-event listener for the whole page', () => {
    const onCoreEvents = vi.fn();
    installBridge({ onCoreEvents });
    const deps = createDefaultDeps();
    deps.subscribe(() => undefined);
    deps.subscribe(() => undefined);
    expect(onCoreEvents).toHaveBeenCalledTimes(1);
  });
});

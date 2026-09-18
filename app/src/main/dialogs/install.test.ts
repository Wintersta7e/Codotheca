import { describe, expect, it, vi } from 'vitest';

import {
  IPC_INSTALL_CANCEL,
  IPC_INSTALL_START,
  type InstallCancelReply,
  type InstallStartReply,
} from '../../shared/channels';
import { registerInstall, type InstallDeps } from './install';

type Handler = (payload: unknown) => Promise<InstallStartReply | InstallCancelReply>;
type RequestMock = ReturnType<typeof vi.fn<InstallDeps['request']>>;

interface Rig {
  start: Handler;
  cancel: Handler;
  request: RequestMock;
}

const resolving = (value: unknown = { runId: 7, refusedBecause: null }): RequestMock =>
  vi.fn(() => Promise.resolve(value));

function throwing(fields: Record<string, unknown>, message: string): RequestMock {
  return vi.fn(() => {
    const err = Object.assign(new Error(message), fields);
    return Promise.reject(err);
  });
}

function rig(request: RequestMock = resolving()): Rig {
  const registered = new Map<string, Handler>();
  registerInstall({
    handle: (channel, fn) => {
      registered.set(channel, fn);
    },
    request,
  });
  const start = registered.get(IPC_INSTALL_START);
  const cancel = registered.get(IPC_INSTALL_CANCEL);
  if (start === undefined || cancel === undefined) throw new Error('a handler was not registered');
  return { start, cancel, request };
}

describe('the install channels', () => {
  it('sends two opaque ids and no path in either direction', async () => {
    const r = rig();
    const reply = await r.start({ projectId: 3, rootId: 9 });

    expect(reply).toEqual({ kind: 'started', start: { runId: 7, refusedBecause: null } });
    expect(r.request).toHaveBeenCalledWith('install.start', { projectId: 3, rootId: 9 });
    // §24.3a: the destination is composed in the core. Nothing that could be a path is sent.
    const [, args] = r.request.mock.calls[0] as [string, Record<string, unknown>];
    expect(Object.keys(args).sort()).toEqual(['projectId', 'rootId']);
  });

  it('carries a refusal back as a reply rather than a failure', async () => {
    // §24.3d: a collision is an answer, not an error. `install.start` returns it inside the value.
    const r = rig(resolving({ runId: null, refusedBecause: 'destination_exists' }));
    const reply = await r.start({ projectId: 3, rootId: 9 });

    expect(reply.kind).toBe('started');
    expect(reply).toEqual({
      kind: 'started',
      start: { runId: null, refusedBecause: 'destination_exists' },
    });
  });

  it('refuses a payload that is not two ids, spawning nothing', async () => {
    const r = rig();
    const bad = [
      { projectId: 3 },
      { rootId: 9 },
      { projectId: 3, rootId: 0 },
      { projectId: 1.5, rootId: 9 },
      { projectId: 3, rootId: '/etc' },
      null,
      'projectId=3',
    ];

    for (const payload of bad) {
      const reply = await r.start(payload);
      expect(reply.kind, JSON.stringify(payload)).toBe('failed');
    }
    expect(r.request).not.toHaveBeenCalled();
  });

  it('reports a core failure with its own code, outcome and retryable flag', async () => {
    const r = rig(throwing({ code: 'CORE_DOWN', outcome: 'unknown', retryable: true }, 'no core'));
    const reply = await r.start({ projectId: 3, rootId: 9 });

    expect(reply).toEqual({
      kind: 'failed',
      error: { code: 'CORE_DOWN', message: 'no core', outcome: 'unknown', retryable: true },
    });
  });

  it('never invents an outcome the core did not state', async () => {
    // §2.2: `null` is "definitely did not take effect". A word here would tell the renderer a
    // clone may have begun when the core has just said it did not.
    const r = rig(throwing({ code: 'PROTOCOL' }, 'bad args'));
    const reply = await r.start({ projectId: 3, rootId: 9 });

    expect(reply).toEqual({
      kind: 'failed',
      error: { code: 'PROTOCOL', message: 'bad args', outcome: null, retryable: false },
    });
  });

  it('cancels by run id alone', async () => {
    const r = rig(resolving({}));
    const reply = await r.cancel({ runId: 7 });

    expect(reply).toEqual({ kind: 'cancelled' });
    expect(r.request).toHaveBeenCalledWith('install.cancel', { runId: 7 });
  });

  it('refuses a cancel that names no run', async () => {
    const r = rig();
    for (const payload of [{ runId: 0 }, { runId: -1 }, {}, null]) {
      const reply = await r.cancel(payload);
      expect(reply.kind, JSON.stringify(payload)).toBe('failed');
    }
    expect(r.request).not.toHaveBeenCalled();
  });

  it('gives the two commands two channels, neither of them the request channel', () => {
    expect(IPC_INSTALL_START).not.toBe(IPC_INSTALL_CANCEL);
  });
});

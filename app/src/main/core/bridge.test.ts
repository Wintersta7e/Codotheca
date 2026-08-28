import { describe, expect, it } from 'vitest';
import { PRIVILEGED_COMMANDS } from '../../generated/protocol';
import { type BridgeReply, IPC_REQUEST } from '../../shared/channels';
import { type BridgeDeps, isRendererCallable, registerBridge } from './bridge';
import { CoreRequestError } from './client';

const KNOWN = ['projects.list', 'projects.launch', 'roots.add'];

interface Rig {
  deps: BridgeDeps;
  called: string[];
  handlers: Map<string, (p: unknown) => Promise<BridgeReply>>;
  sent: { channel: string; payload: unknown }[];
}

function rig(request: BridgeDeps['request']): Rig {
  const called: string[] = [];
  const handlers = new Map<string, (p: unknown) => Promise<BridgeReply>>();
  const sent: { channel: string; payload: unknown }[] = [];
  const deps: BridgeDeps = {
    request: async (name, args) => {
      called.push(name);
      return request(name, args);
    },
    subscribe: () => () => undefined,
    topics: [],
    schedule: () => () => undefined,
    onStatus: () => undefined,
    handle: (c, fn) => {
      handlers.set(c, fn);
    },
    sendToRenderer: (channel, payload) => {
      sent.push({ channel, payload });
    },
    knownCommands: KNOWN,
  };
  return { deps, called, handlers, sent };
}

describe('renderer bridge', () => {
  it('makes no privileged command callable from the renderer', () => {
    expect(PRIVILEGED_COMMANDS.length).toBeGreaterThan(0);
    for (const p of PRIVILEGED_COMMANDS) expect(isRendererCallable(p, KNOWN)).toBe(false);
    expect(isRendererCallable('projects.list', KNOWN)).toBe(true);
    expect(isRendererCallable('nope.notacommand', KNOWN)).toBe(false);
  });

  it('refuses a renderer call for a privileged command before the core is asked', async () => {
    const r = rig(async () => Promise.resolve({}));
    registerBridge(r.deps);
    const handler = r.handlers.get(IPC_REQUEST);
    expect(handler).toBeDefined();
    const reply = await handler?.({ name: 'roots.add', args: { path: '/etc' } });
    expect(reply?.ok).toBe(false);
    expect(reply?.ok === false ? reply.error.code : null).toBe('PROTOCOL');
    expect(r.called).toEqual([]);
  });

  it('crosses a core error as a structured reply, never as a thrown string', async () => {
    const r = rig(() =>
      Promise.reject(new CoreRequestError('CORE_RESTARTED', 'unknown', false, 'lost')),
    );
    registerBridge(r.deps);
    const handler = r.handlers.get(IPC_REQUEST);
    expect(handler).toBeDefined();
    const reply = await handler?.({ name: 'projects.launch', args: {} });
    expect(reply?.ok).toBe(false);
    if (reply?.ok === false) {
      expect(reply.error.code).toBe('CORE_RESTARTED');
      expect(reply.error.outcome).toBe('unknown');
      expect(reply.error.retryable).toBe(false);
    }
  });

  it('passes a permitted command through and returns its value', async () => {
    const r = rig(async () => Promise.resolve({ total: 2 }));
    registerBridge(r.deps);
    const handler = r.handlers.get(IPC_REQUEST);
    expect(handler).toBeDefined();
    const reply = await handler?.({
      name: 'projects.list',
      args: { query: '', sort: 'name', from: 0, to: 1 },
    });
    expect(reply).toEqual({ ok: true, value: { total: 2 } });
    expect(r.called).toEqual(['projects.list']);
  });
});

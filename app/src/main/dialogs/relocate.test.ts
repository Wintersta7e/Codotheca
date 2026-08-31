import { describe, expect, it, vi } from 'vitest';
import { IPC_RELOCATE, type RelocateReply } from '../../shared/channels';
import { pathToBytes, registerRelocateDialog, type RelocateDeps } from './relocate';

type Handler = (payload: unknown) => Promise<RelocateReply>;
type RequestMock = ReturnType<typeof vi.fn<RelocateDeps['request']>>;

interface Rig {
  invoke: Handler;
  request: RequestMock;
  dialog: ReturnType<typeof vi.fn>;
}

const resolving = (): RequestMock => vi.fn(() => Promise.resolve({ id: 4 }));

function throwing(fields: Record<string, unknown>, message: string): RequestMock {
  return vi.fn(() => {
    const err = Object.assign(new Error(message), fields);
    return Promise.reject(err);
  });
}

function rig(chosen: string | null, request: RequestMock = resolving()): Rig {
  const registered: Handler[] = [];
  const dialog = vi.fn(() => Promise.resolve(chosen));
  registerRelocateDialog({
    handle: (channel, fn) => {
      if (channel === IPC_RELOCATE) registered.push(fn);
    },
    showFolderDialog: dialog,
    request,
  });
  const invoke = registered[0];
  if (invoke === undefined) throw new Error('handler was not registered');
  return { invoke, request, dialog };
}

describe('the relocate channel', () => {
  it('sends the dialog path as tagged bytes and never as a string', async () => {
    const r = rig('/srv/work/thing');
    const reply = await r.invoke({ locationId: 4 });
    expect(reply.kind).toBe('relocated');
    expect(r.request).toHaveBeenCalledWith('locations.relocate', {
      locationId: 4,
      pathBytes: pathToBytes('/srv/work/thing'),
    });
    const [, args] = r.request.mock.calls[0] as [string, { pathBytes: { b64: string } }];
    expect(typeof args.pathBytes.b64).toBe('string');
  });

  it('issues no command at all when the user cancels the dialog', async () => {
    const r = rig(null);
    const reply = await r.invoke({ locationId: 4 });
    expect(reply).toEqual({ kind: 'cancelled' });
    expect(r.request).not.toHaveBeenCalled();
  });

  it('refuses a payload that is not a location id, opening no dialog', async () => {
    const r = rig('/srv/work/thing');
    for (const bad of [{ locationId: '../../etc' }, { locationId: 0 }, { locationId: 1.5 }, null]) {
      const reply = await r.invoke(bad);
      expect(reply).toEqual({
        kind: 'failed',
        error: {
          code: 'PROTOCOL',
          message: 'relocate: bad locationId',
          outcome: null,
          retryable: false,
        },
      });
    }
    expect(r.dialog).not.toHaveBeenCalled();
    expect(r.request).not.toHaveBeenCalled();
  });

  it('never accepts a path from the renderer payload', async () => {
    const r = rig('/srv/work/chosen');
    await r.invoke({ locationId: 4, pathBytes: { b64: 'L2V0Yy9wYXNzd2Q=' } });
    const [, args] = r.request.mock.calls[0] as [string, { pathBytes: { b64: string } }];
    expect(args.pathBytes).toEqual(pathToBytes('/srv/work/chosen'));
  });

  it('reports a refusal from the core as a failure, not as a relocation', async () => {
    const r = rig(
      '/srv/work/other',
      throwing({ code: 'REPO_UNREADABLE', retryable: false }, 'lineage mismatch'),
    );
    const reply = await r.invoke({ locationId: 4 });
    expect(reply).toEqual({
      kind: 'failed',
      error: {
        code: 'REPO_UNREADABLE',
        message: 'lineage mismatch',
        // §2.2 has no `failed` outcome: a refusal that did not take effect carries null.
        outcome: null,
        retryable: false,
      },
    });
  });

  it('carries the may-have-landed outcome through rather than flattening it', async () => {
    const r = rig(
      '/srv/work/other',
      throwing({ code: 'CORE_RESTARTED', outcome: 'unknown', retryable: true }, 'core restarted'),
    );
    const reply = await r.invoke({ locationId: 4 });
    expect(reply).toEqual({
      kind: 'failed',
      error: {
        code: 'CORE_RESTARTED',
        message: 'core restarted',
        outcome: 'unknown',
        retryable: true,
      },
    });
  });

  it('names no forgetting: this rewrites a path and removes nothing', () => {
    expect(registerRelocateDialog.toString().toUpperCase()).not.toContain('FORGET');
  });
});

describe('pathToBytes', () => {
  it('round-trips utf-8 through standard base64', () => {
    expect(pathToBytes('/a/b').b64).toBe(Buffer.from('/a/b', 'utf8').toString('base64'));
    expect(Buffer.from(pathToBytes('/a/é/b').b64, 'base64').toString('utf8')).toBe('/a/é/b');
  });
});

import { describe, expect, it, vi } from 'vitest';

import type { CommandName } from '../../generated/protocol';
import { IPC_OPEN_REMOTE_LINK, type OpenRemoteLinkReply } from '../../shared/channels';
import { isOpenableUrl, registerExternalLink, REMOTE_HOST_ALLOWLIST } from './externalLink';

type Handler = (payload: unknown) => Promise<OpenRemoteLinkReply>;

interface Harness {
  readonly invoke: Handler;
  readonly order: string[];
  readonly opened: string[];
  readonly confirmed: string[];
  readonly requests: { name: string; args: unknown }[];
}

function harness(options: {
  url?: unknown;
  accounts?: readonly { host: string }[];
  confirm?: boolean;
  requestThrows?: Error;
  openThrows?: Error;
}): Harness {
  const order: string[] = [];
  const opened: string[] = [];
  const confirmed: string[] = [];
  const requests: { name: string; args: unknown }[] = [];
  let handler: Handler | null = null;

  registerExternalLink({
    handle: (channel, fn) => {
      expect(channel).toBe(IPC_OPEN_REMOTE_LINK);
      handler = fn;
    },
    request: (name: CommandName, args: unknown) => {
      requests.push({ name, args });
      if (name === 'accounts.list') return Promise.resolve(options.accounts ?? []);
      order.push('request');
      if (options.requestThrows !== undefined) return Promise.reject(options.requestThrows);
      return Promise.resolve('url' in options ? options.url : 'https://github.com/acme/widget');
    },
    confirm: (url: string) => {
      order.push('confirm');
      confirmed.push(url);
      return Promise.resolve(options.confirm ?? true);
    },
    openExternal: (url: string) => {
      order.push('openExternal');
      if (options.openThrows !== undefined) return Promise.reject(options.openThrows);
      opened.push(url);
      return Promise.resolve();
    },
  });

  if (handler === null) throw new Error('registerExternalLink registered no handler');
  return { invoke: handler, order, opened, confirmed, requests };
}

const CALL = { projectId: 7, kind: 'repository' };

describe('§25.2: the opener carries no URL inbound and opens only what the core built', () => {
  it('opens the answer the core returned, never a URL the payload carried', async () => {
    const h = harness({ url: 'https://github.com/acme/widget' });
    const reply = await h.invoke({ ...CALL, url: 'https://evil.example.invalid/pwn' });

    expect(reply).toEqual({ kind: 'opened', url: 'https://github.com/acme/widget' });
    expect(h.opened).toEqual(['https://github.com/acme/widget']);
    // The hostile field reached nothing: the only string that was ever considered is the one
    // `remote.webUrl` answered with.
    expect(h.requests.filter((r) => r.name === 'remote.webUrl')).toEqual([
      { name: 'remote.webUrl', args: { projectId: 7, kind: 'repository' } },
    ]);
  });

  it('opens nothing when the core answers NULL', async () => {
    const h = harness({ url: null });
    expect(await h.invoke(CALL)).toEqual({ kind: 'not_linkable' });
    expect(h.opened).toEqual([]);
    expect(h.confirmed).toEqual([]);
  });

  it('opens nothing for a non-https answer', async () => {
    const h = harness({ url: 'http://github.com/acme/widget' });
    expect(await h.invoke(CALL)).toEqual({ kind: 'not_linkable' });
    expect(h.opened).toEqual([]);
  });

  it('opens nothing for an answer on a host outside the allowlist', async () => {
    const h = harness({ url: 'https://evil.example.invalid/acme/widget' });
    expect(await h.invoke(CALL)).toEqual({ kind: 'not_linkable' });
    expect(h.opened).toEqual([]);
  });

  it('opens nothing when the confirmation is declined', async () => {
    const h = harness({ confirm: false });
    expect(await h.invoke(CALL)).toEqual({ kind: 'declined' });
    expect(h.opened).toEqual([]);
  });

  it('confirms before it opens, on every accepted path', async () => {
    const h = harness({});
    await h.invoke(CALL);
    expect(h.order).toEqual(['request', 'confirm', 'openExternal']);
  });

  it('refuses a payload that is not a project id and a kind', async () => {
    const h = harness({});
    for (const bad of [
      null,
      'nope',
      {},
      { projectId: 7 },
      { kind: 'repository' },
      { projectId: -1, kind: 'issues' },
    ]) {
      const reply = await h.invoke(bad);
      expect(reply.kind).toBe('failed');
    }
    expect(h.requests.filter((r) => r.name === 'remote.webUrl')).toEqual([]);
  });

  it('reports a refused command as a failure and opens nothing', async () => {
    const refusal = Object.assign(new Error('unknown kind'), { code: 'PROTOCOL' });
    const h = harness({ requestThrows: refusal });
    const reply = await h.invoke({ projectId: 7, kind: 'not-a-kind' });
    expect(reply.kind).toBe('failed');
    expect(h.opened).toEqual([]);
  });

  it('admits an Enterprise host only when an account row names it', async () => {
    const stranger = harness({ url: 'https://forge.example.invalid/acme/widget' });
    expect(await stranger.invoke(CALL)).toEqual({ kind: 'not_linkable' });

    const connected = harness({
      url: 'https://forge.example.invalid/acme/widget',
      accounts: [{ host: 'forge.example.invalid' }],
    });
    expect(await connected.invoke(CALL)).toEqual({
      kind: 'opened',
      url: 'https://forge.example.invalid/acme/widget',
    });

    const other = harness({
      url: 'https://other.example.invalid/acme/widget',
      accounts: [{ host: 'forge.example.invalid' }],
    });
    expect(await other.invoke(CALL)).toEqual({ kind: 'not_linkable' });
  });
});

describe('§25.2: the shape a returned URL must have before it reaches the OS', () => {
  const allowed = [...REMOTE_HOST_ALLOWLIST];

  it('accepts the five link shapes and nothing else', () => {
    for (const url of [
      'https://github.com/acme/widget',
      'https://github.com/acme/widget/issues',
      'https://github.com/acme/widget/pulls',
      'https://github.com/acme/widget/actions',
      'https://github.com/acme/widget/releases',
    ]) {
      expect(isOpenableUrl(url, allowed), url).toBe(true);
    }
    for (const url of [
      'https://github.com/acme/widget/settings',
      'https://github.com/acme',
      'https://github.com/acme/widget/issues/1',
      'https://user:pw@github.com/acme/widget',
      'https://github.com:8443/acme/widget',
      'https://github.com/acme/widget?x=1',
      'https://github.com/acme/widget#frag',
      'javascript:alert(1)',
      'not a url',
    ]) {
      expect(isOpenableUrl(url, allowed), url).toBe(false);
    }
  });
});

describe('the handler is registered on one channel', () => {
  it('registers exactly one handler, on IPC_OPEN_REMOTE_LINK', () => {
    const handle = vi.fn();
    registerExternalLink({
      handle,
      request: () => Promise.resolve(null),
      confirm: () => Promise.resolve(true),
      openExternal: () => Promise.resolve(),
    });
    expect(handle).toHaveBeenCalledTimes(1);
    expect(handle.mock.calls[0]?.[0]).toBe(IPC_OPEN_REMOTE_LINK);
  });
});

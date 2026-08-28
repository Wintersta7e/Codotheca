import { describe, expect, it } from 'vitest';
import { NON_IDEMPOTENT_COMMANDS } from '../../generated/protocol';
import { CoreClient, CoreRequestError, type SupervisorLike } from './client';
import { COMMAND_EFFECT, isNonIdempotent } from './idempotence';
import type { CoreStatus } from './supervisor';
import type { Inbound, Outbound } from './wire';

class FakeSupervisor implements SupervisorLike {
  status: CoreStatus = { kind: 'ready', epoch: 1, coreVersion: '0', protocolVersion: 1, pid: 2 };
  currentEpoch = 1;
  sent: Inbound[] = [];
  private statusCb: ((s: CoreStatus) => void) | null = null;
  private frameCb: ((f: Outbound) => void) | null = null;
  private deathCb: ((e: number) => void) | null = null;
  onStatus(fn: (s: CoreStatus) => void): void {
    this.statusCb = fn;
  }
  onFrame(fn: (f: Outbound) => void): void {
    this.frameCb = fn;
  }
  onEpochEnd(fn: (e: number) => void): void {
    this.deathCb = fn;
  }
  send(frame: Inbound): boolean {
    this.sent.push(frame);
    return true;
  }
  die(): void {
    this.deathCb?.(this.currentEpoch);
  }
  emit(f: Outbound): void {
    this.frameCb?.(f);
  }
  ready(epoch: number): void {
    this.currentEpoch = epoch;
    this.status = { kind: 'ready', epoch, coreVersion: '0', protocolVersion: 1, pid: 2 };
    this.statusCb?.(this.status);
  }
}

describe('command effects', () => {
  it('calls launch a write and list a read', () => {
    expect(COMMAND_EFFECT['projects.launch']).toBe('write');
    expect(COMMAND_EFFECT['projects.list']).toBe('read');
    expect(isNonIdempotent('projects.launch')).toBe(true);
    expect(isNonIdempotent('projects.list')).toBe(false);
  });

  // The schema owns `idempotent`; this table is a strictly wider "does it write at all". The
  // dangerous drift is one direction only — the schema forbidding a replay this table would
  // allow — so that is what is asserted, by reading the generated set.
  it('classifies every command the schema calls non-idempotent as a write', () => {
    for (const name of NON_IDEMPOTENT_COMMANDS) {
      expect(COMMAND_EFFECT[name], `${name} is non-idempotent in the schema`).toBe('write');
    }
  });
});

describe('core client', () => {
  it('reports a pending launch across a core death as unknown, not retryable, not replayed', async () => {
    const sup = new FakeSupervisor();
    const client = new CoreClient(sup);
    const p = client.request('projects.launch', {
      projectId: 'p' as never,
      locationId: 'l' as never,
      targetId: 't' as never,
    });
    const before = sup.sent.length;
    sup.die();
    const e = await p.then(
      () => null,
      (err: unknown) => err,
    );
    expect(e).toBeInstanceOf(CoreRequestError);
    expect((e as CoreRequestError).code).toBe('CORE_RESTARTED');
    expect((e as CoreRequestError).outcome).toBe('unknown');
    expect((e as CoreRequestError).retryable).toBe(false);
    expect(sup.sent.length).toBe(before);
  });

  it('reports a pending read across a core death as retryable', async () => {
    const sup = new FakeSupervisor();
    const client = new CoreClient(sup);
    const p = client.request('projects.get', { id: 'p' as never });
    sup.die();
    const e = await p.then(
      () => null,
      (err: unknown) => err,
    );
    expect(e).toBeInstanceOf(CoreRequestError);
    expect((e as CoreRequestError).outcome).toBeNull();
    expect((e as CoreRequestError).retryable).toBe(true);
  });

  it('resolves the request a response names', async () => {
    const sup = new FakeSupervisor();
    const client = new CoreClient(sup);
    const p = client.request('projects.peek', { id: 'p' as never });
    const sentId = sup.sent.filter((f) => f.t === 'request').at(-1);
    expect(sentId?.t).toBe('request');
    if (sentId?.t !== 'request') throw new Error('unreachable');
    sup.emit({ t: 'response', epoch: 1, id: sentId.id, ok: { name: 'x' } });
    await expect(p).resolves.toEqual({ name: 'x' });
  });

  it('asks for a fresh snapshot on a gap instead of delivering', () => {
    const sup = new FakeSupervisor();
    const client = new CoreClient(sup);
    const seen: string[] = [];
    client.subscribe('projects', {
      onEvent: (ev) => seen.push(ev),
      onSnapshot: () => seen.push('snapshot'),
    });
    sup.emit({ t: 'event', epoch: 1, topic: 'projects', event: 'upserted', seq: 1, data: {} });
    sup.emit({ t: 'event', epoch: 1, topic: 'projects', event: 'upserted', seq: 5, data: {} });
    expect(seen).toEqual(['upserted']);
    expect(sup.sent.some((f) => f.t === 'resync' && f.topic === 'projects')).toBe(true);
  });

  it('resubscribes and resynchronises on a new epoch', () => {
    const sup = new FakeSupervisor();
    const client = new CoreClient(sup);
    client.subscribe('scan', { onEvent: () => undefined, onSnapshot: () => undefined });
    sup.sent.length = 0;
    sup.die();
    sup.ready(2);
    expect(sup.sent.some((f) => f.t === 'subscribe' && f.topic === 'scan')).toBe(true);
    expect(sup.sent.some((f) => f.t === 'resync' && f.topic === 'scan')).toBe(true);
  });
});

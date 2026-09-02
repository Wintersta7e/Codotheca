import { expect, test } from 'vitest';

import type { CommandName } from '../../src/generated/protocol';
import { NON_IDEMPOTENT_COMMANDS } from '../../src/generated/protocol';
import { CoreClient, CoreRequestError, type SupervisorLike } from '../../src/main/core/client';
import type { CoreStatus } from '../../src/main/core/supervisor';
import type { Inbound, Outbound } from '../../src/main/core/wire';

/**
 * Criterion 24's shell half: killing the core loses no committed data and replays nothing
 * non-idempotent.
 *
 * The shell cannot observe the durability half — that is the core's, over a real scan — but it
 * owns the two clauses §2.2 states about the wire: every request in flight when the child dies
 * is rejected with `CORE_RESTARTED`, and a write is never re-sent on the new connection just
 * because its outcome is unknown. "Unknown" is the honest answer and a replay would turn it into
 * a second launch.
 *
 * These names are the join key. `acceptance/criteria.json` registers them verbatim as the `test`
 * field of two `automated` checks, and the acceptance gate fails an automated check whose test
 * did not run — so a rename is reported rather than silently un-covering the criterion. There is
 * deliberately no `describe` around them: vitest's `fullName` prefixes every enclosing suite
 * title, and the registry joins on that string.
 */
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

/**
 * The criterion is about *every* command, so the command name is data here rather than a literal
 * and its arguments cannot be typed per call. The client never reads the arguments — it forwards
 * them into a frame — so an empty object exercises the same path a real one does.
 */
type AnyRequest = (name: CommandName, args: unknown) => Promise<unknown>;

function send(client: CoreClient, name: CommandName): Promise<unknown> {
  return (client.request as unknown as AnyRequest)(name, {});
}

/**
 * The node project raises the default timeout to five minutes for the corpus builder. These two
 * tests fail by *hanging* when the client replays instead of rejecting — nothing ever settles —
 * so they carry their own short deadline rather than holding a CI job for five minutes to report
 * a regression that is visible in ten seconds.
 */
const DEADLINE_MS = 10_000;

/** Attach the rejection handler in the same turn the request is made, or Node reports it. */
function settle(promise: Promise<unknown>): Promise<unknown> {
  return promise.then(
    () => null,
    (error: unknown) => error,
  );
}

test(
  'AC-24 every pending request is rejected with CORE_RESTARTED',
  async () => {
    const sup = new FakeSupervisor();
    const client = new CoreClient(sup);

    const reads: CommandName[] = ['projects.list', 'projects.peek', 'scan.status'];
    const writes = [...NON_IDEMPOTENT_COMMANDS];
    const inFlight = [...writes, ...reads].map((name) => ({
      name,
      settled: settle(send(client, name)),
    }));
    expect(inFlight.length).toBeGreaterThan(1);

    sup.die();

    for (const { name, settled } of inFlight) {
      const error = await settled;
      expect(error, `${name} must reject rather than hang`).toBeInstanceOf(CoreRequestError);
      const e = error as CoreRequestError;
      expect(e.code, name).toBe('CORE_RESTARTED');
      if ((NON_IDEMPOTENT_COMMANDS as readonly string[]).includes(name)) {
        // It may or may not have run. Saying so is the whole point; a retry would launch twice.
        expect(e.outcome, name).toBe('unknown');
        expect(e.retryable, name).toBe(false);
      } else {
        expect(e.outcome, name).toBeNull();
        expect(e.retryable, name).toBe(true);
      }
    }

    // The map is drained, not merely walked: a second death must find nothing left to reject, or
    // a later restart would settle an already-settled promise.
    sup.ready(2);
    sup.die();
  },
  DEADLINE_MS,
);

test(
  'AC-24 nothing non-idempotent is replayed across an epoch',
  async () => {
    const sup = new FakeSupervisor();
    const client = new CoreClient(sup);
    client.subscribe('scan', { onEvent: () => undefined, onSnapshot: () => undefined });

    const settled = NON_IDEMPOTENT_COMMANDS.map((name) => settle(send(client, name)));
    const sentBefore = sup.sent.filter((f) => f.t === 'request').length;
    expect(sentBefore).toBe(NON_IDEMPOTENT_COMMANDS.length);

    sup.die();
    sup.sent.length = 0;
    sup.ready(2);
    await Promise.all(settled);

    const replayed = sup.sent.filter(
      (f) =>
        f.t === 'request' && (NON_IDEMPOTENT_COMMANDS as readonly string[]).includes(f.command),
    );
    expect(replayed).toEqual([]);

    // The connection is new, and the resubscribe rides the new epoch rather than the dead one.
    expect(sup.currentEpoch).toBe(2);
    expect(sup.sent.some((f) => f.t === 'subscribe' && f.topic === 'scan')).toBe(true);
    expect(sup.sent.some((f) => f.t === 'resync' && f.topic === 'scan')).toBe(true);

    // A frame carrying the dead epoch is dropped rather than delivered against the new stream.
    const seen: string[] = [];
    client.subscribe('scan', {
      onEvent: (event) => seen.push(event),
      onSnapshot: () => seen.push('snapshot'),
    });
    sup.emit({ t: 'event', epoch: 1, topic: 'scan', event: 'progress', seq: 1, data: {} });
    expect(seen).toEqual([]);
  },
  DEADLINE_MS,
);

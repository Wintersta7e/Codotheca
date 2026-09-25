/**
 * Request/response and subscriptions over one supervised core.
 *
 * On core death every pending request is rejected immediately. A read is retryable; a write
 * is `unknown` and is never replayed, because it may have completed before the response was
 * lost and replaying a launch opens the editor twice.
 */
import type {
  CommandArgs,
  CommandName,
  CommandResult,
  ErrorCode,
  Topic,
} from '../../generated/protocol';
import { isNonIdempotent } from './idempotence';
import { type StreamState, acceptFrame, newStreamState } from './streams';
import type { CoreStatus } from './supervisor';
import type { EventFrame, Inbound, Outbound, Outcome, SnapshotFrame } from './wire';

/** Everything `CoreClient` needs from a supervisor. `CoreSupervisor` satisfies it. */
export interface SupervisorLike {
  readonly status: CoreStatus;
  readonly currentEpoch: number;
  onStatus(fn: (s: CoreStatus) => void): void;
  onFrame(fn: (f: Outbound) => void): void;
  onEpochEnd(fn: (epoch: number) => void): void;
  send(frame: Inbound): boolean;
}

export class CoreRequestError extends Error {
  readonly code: ErrorCode;
  /** `null` — definitely did not take effect. `'unknown'` — may have. */
  readonly outcome: Outcome | null;
  readonly retryable: boolean;

  constructor(code: ErrorCode, outcome: Outcome | null, retryable: boolean, message: string) {
    super(message);
    this.code = code;
    this.outcome = outcome;
    this.retryable = retryable;
    this.name = 'CoreRequestError';
  }
}

interface Pending {
  command: CommandName;
  resolve: (v: unknown) => void;
  reject: (e: CoreRequestError) => void;
}

export interface TopicHandler {
  onEvent: (event: string, data: unknown) => void;
  onSnapshot: (data: unknown) => void;
}

export class CoreClient {
  private readonly sup: SupervisorLike;
  private nextId = 1;
  private readonly pending = new Map<number, Pending>();
  private readonly handlers = new Map<Topic, Set<TopicHandler>>();
  private readonly streams = new Map<Topic, StreamState>();

  constructor(sup: SupervisorLike) {
    this.sup = sup;
    sup.onFrame((f) => {
      this.route(f);
    });
    sup.onEpochEnd(() => {
      this.rejectAllPending();
    });
    sup.onStatus((s) => {
      if (s.kind === 'ready') this.resubscribeAll(s.epoch);
    });
  }

  request<K extends CommandName>(name: K, args: CommandArgs[K]): Promise<CommandResult[K]> {
    return new Promise<CommandResult[K]>((resolve, reject) => {
      const id = this.nextId;
      this.nextId += 1;
      this.pending.set(id, { command: name, resolve: resolve as (v: unknown) => void, reject });
      const sent = this.sup.send({ t: 'request', id, command: name, args });
      if (!sent && this.sup.status.kind === 'failed') {
        this.pending.delete(id);
        reject(this.restartError(name));
      }
    });
  }

  subscribe(topic: Topic, handler: TopicHandler): () => void {
    let set = this.handlers.get(topic);
    if (set === undefined) {
      set = new Set<TopicHandler>();
      this.handlers.set(topic, set);
      this.streams.set(topic, newStreamState(this.sup.currentEpoch));
      this.sup.send({ t: 'subscribe', topic });
    }
    const members = set;
    members.add(handler);
    return (): void => {
      members.delete(handler);
      if (members.size === 0) {
        this.handlers.delete(topic);
        this.streams.delete(topic);
        this.sup.send({ t: 'unsubscribe', topic });
      }
    };
  }

  private route(f: Outbound): void {
    if (f.t === 'response' || f.t === 'error') {
      const p = this.pending.get(f.id);
      if (p === undefined) return;
      this.pending.delete(f.id);
      if (f.t === 'response') p.resolve(f.ok);
      else p.reject(new CoreRequestError(f.code, f.outcome, f.outcome === null, f.message));
      return;
    }
    if (f.t === 'event' || f.t === 'snapshot') this.routeTopic(f);
  }

  private routeTopic(f: EventFrame | SnapshotFrame): void {
    const state = this.streams.get(f.topic);
    const set = this.handlers.get(f.topic);
    if (state === undefined || set === undefined) return;
    const result = acceptFrame(state, f);
    if (result.kind === 'gap') {
      this.sup.send({ t: 'resync', topic: f.topic });
      return;
    }
    if (result.kind === 'stale') return;
    if (result.kind === 'snapshot') {
      for (const h of set) h.onSnapshot(f.data);
      return;
    }
    if (f.t === 'event') for (const h of set) h.onEvent(f.event, f.data);
  }

  /** Subscriptions do not survive an epoch: resubscribe and resynchronise. */
  private resubscribeAll(epoch: number): void {
    for (const topic of this.handlers.keys()) {
      this.streams.set(topic, newStreamState(epoch));
      this.sup.send({ t: 'subscribe', topic });
      this.sup.send({ t: 'resync', topic });
    }
  }

  private rejectAllPending(): void {
    const dead = [...this.pending.values()];
    this.pending.clear();
    for (const p of dead) p.reject(this.restartError(p.command));
  }

  private restartError(command: CommandName): CoreRequestError {
    return isNonIdempotent(command)
      ? new CoreRequestError('CORE_RESTARTED', 'unknown', false, `${command} outcome unknown`)
      : new CoreRequestError('CORE_RESTARTED', null, true, `${command} was not executed`);
  }
}

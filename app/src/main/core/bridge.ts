/**
 * The renderer's only door to the core.
 *
 * The renderer may never originate a filesystem path or an executable, so the commands that
 * carry one are refused here outright. They are issued by the shell after a native dialog it
 * owns, on a different channel.
 */
import {
  type CommandName,
  type EventPayloads,
  PRIVILEGED_COMMANDS,
  type Topic,
  type TopicEvents,
} from '../../generated/protocol';
import {
  type BridgeCall,
  type BridgeReply,
  IPC_CORE_STATUS,
  IPC_CORE_STATUS_NOW,
  IPC_EVENTS,
  IPC_REQUEST,
  type RendererEvent,
} from '../../shared/channels';
import { CoreRequestError, type SnapshotPosition, type TopicHandler } from './client';
import { COALESCE_WINDOW_MS, createCoalescer } from './coalesce';
import type { CoreStatus } from './supervisor';

const PRIVILEGED = new Set<string>(PRIVILEGED_COMMANDS);

export type BridgeRequest = (name: CommandName, args: unknown) => Promise<unknown>;

export interface BridgeDeps {
  request: BridgeRequest;
  subscribe: (topic: Topic, handler: TopicHandler) => () => void;
  topics: Topic[];
  /** Returns its own cancel function; see `createCoalescer`. */
  schedule: (fn: () => void, ms: number) => () => void;
  onStatus: (fn: (s: CoreStatus) => void) => void;
  /** The status as `onStatus` would deliver it now, for a renderer that mounted late. */
  statusNow: () => CoreStatus;
  handle: (channel: string, fn: (payload: unknown) => Promise<BridgeReply>) => void;
  sendToRenderer: (channel: string, payload: unknown) => void;
  knownCommands: readonly string[];
}

export function isRendererCallable(name: string, known: readonly string[]): name is CommandName {
  return known.includes(name) && !PRIVILEGED.has(name);
}

function refuse(message: string): BridgeReply {
  return { ok: false, error: { code: 'PROTOCOL', message, outcome: null, retryable: false } };
}

export function registerBridge(deps: BridgeDeps): void {
  deps.handle(IPC_REQUEST, async (payload: unknown): Promise<BridgeReply> => {
    if (typeof payload !== 'object' || payload === null) return refuse('malformed call');
    const call = payload as Partial<BridgeCall>;
    if (typeof call.name !== 'string') return refuse('malformed call');
    if (!isRendererCallable(call.name, deps.knownCommands)) {
      return refuse(`${call.name} is not callable from the renderer`);
    }
    try {
      return { ok: true, value: await deps.request(call.name, call.args) };
    } catch (e: unknown) {
      if (e instanceof CoreRequestError) {
        return {
          ok: false,
          error: { code: e.code, message: e.message, outcome: e.outcome, retryable: e.retryable },
        };
      }
      return {
        ok: false,
        error: { code: 'INTERNAL', message: String(e), outcome: null, retryable: false },
      };
    }
  });

  deps.onStatus((s) => {
    deps.sendToRenderer(IPC_CORE_STATUS, s);
  });
  deps.handle(IPC_CORE_STATUS_NOW, () => Promise.resolve({ ok: true, value: deps.statusNow() }));

  // One IPC message per frame, never one per event: a scan emitting thousands of individual
  // messages janks Electron regardless of encoding.
  const coalescer = createCoalescer<RendererEvent>({
    windowMs: COALESCE_WINDOW_MS,
    sink: (batch) => {
      deps.sendToRenderer(IPC_EVENTS, batch);
    },
    schedule: deps.schedule,
  });
  for (const topic of deps.topics) {
    deps.subscribe(topic, {
      onEvent: (event, data) => {
        coalescer.push({ topic, event, data });
      },
      // A snapshot replaces every delta it covers — the answer to a resync after a gap, or to a
      // queue the core collapsed — so a view that misses it stays stale until it asks again.
      onSnapshot: (data, at) => {
        const payload = snapshotEvent(topic, data, at);
        if (payload !== undefined) coalescer.push({ topic, event: 'snapshot', data: payload });
      },
    });
  }
}

/** A topic whose schema declares a `snapshot` event. */
type SnapshotTopic = { [T in Topic]: 'snapshot' extends TopicEvents[T] ? T : never }[Topic];

/**
 * Whether each snapshot event's type carries the stream position. Typed from the generated
 * schema, so a topic gaining a snapshot, or a snapshot type gaining or losing its position, does
 * not compile until this says so.
 */
const CARRIES_POSITION: {
  readonly [T in SnapshotTopic]: 'throughSeq' extends keyof EventPayloads[`${T}/snapshot`]
    ? true
    : false;
} = { core: true, projects: true, install: false, sync: false };

function declaresSnapshot(topic: Topic): topic is SnapshotTopic {
  return Object.hasOwn(CARRIES_POSITION, topic);
}

/**
 * A snapshot frame's data as the `<topic>/snapshot` event the schema declares, or `undefined`
 * when there is nothing to send.
 */
function snapshotEvent(topic: Topic, data: unknown, at: SnapshotPosition): unknown {
  if (!declaresSnapshot(topic)) return undefined;
  const state = typeof data === 'object' && data !== null ? data : null;
  // `null` is a snapshot the core could not compute. The core topic's crosses anyway: it is the
  // only place the renderer learns first run's stamp and git's version, and it answers the
  // subscription once startup — a rebuild included — is over, so its arrival is the news. Any
  // other topic's has no state to replace, and the shelf would read one as no projects at all.
  if (state === null) return topic === 'core' ? { ...at } : undefined;
  return CARRIES_POSITION[topic] ? { ...state, ...at } : state;
}

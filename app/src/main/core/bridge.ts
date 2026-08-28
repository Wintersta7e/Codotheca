/**
 * The renderer's only door to the core.
 *
 * The renderer may never originate a filesystem path or an executable, so the commands that
 * carry one are refused here outright. They are issued by the shell after a native dialog it
 * owns, on a different channel.
 */
import { type CommandName, PRIVILEGED_COMMANDS, type Topic } from '../../generated/protocol';
import {
  type BridgeCall,
  type BridgeReply,
  IPC_CORE_STATUS,
  IPC_EVENTS,
  IPC_REQUEST,
  type RendererEvent,
} from '../../shared/channels';
import { CoreRequestError } from './client';
import { COALESCE_WINDOW_MS, createCoalescer } from './coalesce';
import type { CoreStatus } from './supervisor';

const PRIVILEGED = new Set<string>(PRIVILEGED_COMMANDS);

export type BridgeRequest = (name: CommandName, args: unknown) => Promise<unknown>;

export interface BridgeDeps {
  request: BridgeRequest;
  subscribe: (topic: Topic, onEvent: (event: string, data: unknown) => void) => () => void;
  topics: Topic[];
  /** Returns its own cancel function; see `createCoalescer`. */
  schedule: (fn: () => void, ms: number) => () => void;
  onStatus: (fn: (s: CoreStatus) => void) => void;
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
    deps.subscribe(topic, (event, data) => {
      coalescer.push({ topic, event, data });
    });
  }
}

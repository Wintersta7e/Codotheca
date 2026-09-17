/**
 * The project page's only door to the outside. Every command, every event and every clock read
 * on this surface goes through one injected object, so the whole page is renderable in jsdom
 * with no Electron and no core behind it.
 *
 * §2.4: the core's `message` is diagnostic and is never shown raw. `CommandError` carries the
 * closed code; the prose for it is written at the call site, in the shell's voice.
 */
import { createContext, useContext } from 'react';

import type {
  CommandArgs,
  CommandName,
  CommandResult,
  ErrorCode,
  LocationId,
  Outcome,
  ProjectId,
  RemoteLinkKind,
} from '../../generated/protocol';
import type {
  BridgeError,
  BridgeReply,
  OpenRemoteLinkReply,
  RelocateReply,
  RendererEvent,
} from '../../shared/channels';

/**
 * §2.2's three fields, carried together. `outcome` is the generated `Outcome | null` and not a
 * two-word union: `null` is "definitely did not take effect" and `'unknown'` is "may have", and
 * a retry decision needs to tell those apart.
 */
export class CommandError extends Error {
  readonly code: ErrorCode;
  readonly outcome: Outcome | null;
  readonly retryable: boolean;

  constructor(error: BridgeError) {
    super(error.message);
    this.name = 'CommandError';
    this.code = error.code;
    this.outcome = error.outcome;
    this.retryable = error.retryable;
  }
}

export interface ProjectPageDeps {
  request: <K extends CommandName>(name: K, args: CommandArgs[K]) => Promise<CommandResult[K]>;
  /** Privileged (§2.4): the shell owns the folder dialog, the renderer originates no path. */
  relocate: (locationId: LocationId) => Promise<RelocateReply>;
  /**
   * §25.2: an id and a link kind, never a URL. The shell asks the core for the string, checks it
   * again, confirms it with the user by name, and opens it.
   */
  openRemoteLink: (projectId: ProjectId, kind: RemoteLinkKind) => Promise<OpenRemoteLinkReply>;
  subscribe: (handler: (event: RendererEvent) => void) => () => void;
  /** Unix seconds. */
  now: () => number;
}

export const ProjectPageDepsContext = createContext<ProjectPageDeps | null>(null);

export function useProjectPageDeps(): ProjectPageDeps {
  const deps = useContext(ProjectPageDepsContext);
  if (deps === null) {
    throw new Error('ProjectPageDepsContext is not provided');
  }
  return deps;
}

/**
 * Shape, not value. The shell is the only producer on this channel and the topic vocabulary is
 * the generated one; what this guards against is a malformed batch, not a hostile one.
 */
function isRendererEvent(value: unknown): value is RendererEvent {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Record<string, unknown>;
  return typeof candidate['topic'] === 'string' && typeof candidate['event'] === 'string';
}

/**
 * `onCoreEvents` has no disposer, so it is registered once and fanned out here. Lazily, because
 * a page that never mounts must not install a listener at import time.
 */
export function createEventFanout(
  register: (cb: (batch: unknown) => void) => void,
): (handler: (event: RendererEvent) => void) => () => void {
  const handlers = new Set<(event: RendererEvent) => void>();
  let registered = false;

  return (handler) => {
    if (!registered) {
      registered = true;
      register((batch: unknown) => {
        if (!Array.isArray(batch)) return;
        for (const item of batch as unknown[]) {
          if (!isRendererEvent(item)) continue;
          for (const fn of [...handlers]) {
            try {
              fn(item);
            } catch {
              // One subscriber's failure may not silence the rest of the page.
            }
          }
        }
      });
    }
    handlers.add(handler);
    return () => {
      handlers.delete(handler);
    };
  };
}

export function createDefaultDeps(): ProjectPageDeps {
  const bridge = window.codotheca;
  const subscribe = createEventFanout((cb) => {
    bridge.onCoreEvents(cb);
  });

  return {
    request: async <K extends CommandName>(name: K, args: CommandArgs[K]) => {
      const reply = (await bridge.request(name, args)) as BridgeReply;
      if (!reply.ok) throw new CommandError(reply.error);
      return reply.value as CommandResult[K];
    },
    relocate: (locationId) => bridge.relocate(locationId),
    openRemoteLink: (projectId, kind) => bridge.openRemoteLink(projectId, kind),
    subscribe,
    now: () => Math.floor(Date.now() / 1000),
  };
}

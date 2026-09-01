/**
 * The renderer's one path to the core. Every read crosses the protocol (§1.10, §2.4) — the
 * renderer is sandboxed and holds no database handle — so this wrapper is the whole surface.
 *
 * G4: three renderer RPC wrappers already exist (plan 15's `CoreRpc`, plan 16's `requestCore`,
 * plan 14's `ProjectPageDeps.request`). This is the one that is not bound to a feature folder,
 * and plan 15's `CoreRpc` is an interface `{ request: call }` satisfies structurally rather
 * than a second implementation. Their files are not edited here; the collision is reported.
 */
import type {
  CommandArgs,
  CommandName,
  CommandResult,
  ErrorCode,
  Outcome,
} from '../../generated/protocol.js';
import type { BridgeError, BridgeReply } from '../../shared/channels.js';

export type CoreCall = <K extends CommandName>(
  name: K,
  args: CommandArgs[K],
) => Promise<CommandResult[K]>;

/**
 * §2.4: the core's `message` is diagnostic and is never shown raw. It is kept on `detail` for
 * the rolling log; `message` carries the command and the code and nothing a path could hide in.
 *
 * `outcome` is the wire type, not a widened string: §2.2 gives `null` (definitely did not take
 * effect) and `'unknown'` (may have) as different answers, and the renderer decides whether a
 * retry is safe on exactly that difference.
 */
export class CallFailure extends Error {
  readonly code: ErrorCode;
  readonly outcome: Outcome | null;
  readonly retryable: boolean;
  readonly detail: string;

  constructor(command: string, error: BridgeError) {
    super(`${command} failed: ${error.code}`);
    this.name = 'CallFailure';
    this.code = error.code;
    this.outcome = error.outcome;
    this.retryable = error.retryable;
    this.detail = error.message;
  }
}

/** The shell's own channels answer the same envelope, so they unwrap the same way. */
export function unwrapReply<T>(reply: BridgeReply): T {
  if (reply.ok) return reply.value as T;
  throw new CallFailure('shell', reply.error);
}

export function callWith(request: (name: string, args: unknown) => Promise<BridgeReply>): CoreCall {
  return async <K extends CommandName>(name: K, args: CommandArgs[K]) => {
    const reply = await request(name, args);
    if (reply.ok) return reply.value as CommandResult[K];
    throw new CallFailure(name, reply.error);
  };
}

export const call: CoreCall = callWith(
  (name, args) => window.codotheca.request(name, args) as Promise<BridgeReply>,
);

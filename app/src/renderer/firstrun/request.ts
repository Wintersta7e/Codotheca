import type { CommandArgs, CommandName, CommandResult, ErrorCode } from '../../generated/protocol';

/** A refusal from the core, carrying its closed-enum code. */
export class CoreCallError extends Error {
  readonly code: ErrorCode;

  constructor(code: ErrorCode, message: string) {
    super(message);
    this.name = 'CoreCallError';
    this.code = code;
  }
}

interface BridgeShape {
  request(name: string, args: unknown): Promise<unknown>;
}

function bridge(): BridgeShape {
  const found = (globalThis as { codotheca?: BridgeShape }).codotheca;
  if (found === undefined) {
    throw new CoreCallError('INTERNAL', 'the bridge is not mounted');
  }
  return found;
}

/**
 * One command, typed. The core's `message` is diagnostic and is never shown raw (§2.4); the
 * screens map `code` to their own prose.
 */
export async function requestCore<K extends CommandName>(
  name: K,
  args: CommandArgs[K],
): Promise<CommandResult[K]> {
  const reply = (await bridge().request(name, args)) as
    { ok: true; value: unknown } | { ok: false; error: { code: ErrorCode; message: string } };
  if (!reply.ok) {
    throw new CoreCallError(reply.error.code, reply.error.message);
  }
  return reply.value as CommandResult[K];
}

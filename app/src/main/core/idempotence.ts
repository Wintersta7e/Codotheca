/**
 * Which commands may be replayed after the core dies, and which may not.
 */
import type { CommandName } from '../../generated/protocol';

export type CommandEffect = 'read' | 'write';

/**
 * `read` iff the command performs no write of any kind and spawns no process. Everything else
 * is `write` and is never auto-replayed: replaying a launch opens the editor twice.
 *
 * This is deliberately wider than the schema's `idempotent` flag, which asks whether re-running
 * yields the same result. Erring toward `write` costs a user-visible refusal; erring toward
 * `read` acts twice. `client.test.ts` asserts every command the schema calls non-idempotent is
 * `write` here, so the two can only diverge in the safe direction.
 *
 * `Record<CommandName, …>` makes an unclassified command a type error, so a command added to
 * the schema cannot reach the wire without this decision being made. If tsc reports a missing
 * or excess key, the schema is the authority — add or delete the row and classify it by the
 * rule above.
 */
export const COMMAND_EFFECT: Record<CommandName, CommandEffect> = {
  'app.hello_ack': 'read',
  'app.shutdown': 'write',
  'roots.suggest': 'read',
  'roots.list': 'read',
  'roots.add': 'write',
  'roots.remove': 'write',
  'roots.setEnabled': 'write',
  'roots.setDescend': 'write',
  'scan.start': 'write',
  'scan.cancel': 'write',
  'scan.status': 'read',
  'problems.list': 'read',
  'settings.get': 'read',
  'settings.set': 'write',
  'view.get': 'read',
  'view.set': 'write',
  'diag.bundle': 'write',
  'projects.list': 'read',
  'projects.get': 'read',
  'projects.peek': 'read',
  'projects.setFlags': 'write',
  'projects.setNote': 'write',
  'projects.merge': 'write',
  'projects.unmergeHint': 'read',
  'projects.requeue': 'write',
  'projects.launch': 'write',
  'locations.setTrusted': 'write',
  'locations.relocate': 'write',
  // [p2] §24.7: read-only and unprivileged, but it fetches — so it is not callable on hover.
  'locations.uninstallPreflight': 'read',
  // [p2] §24.8: the one phase-2 command that removes a user's working copy.
  'locations.uninstall': 'write',
  'targets.list': 'read',
  'targets.setDefault': 'write',
  'targets.upsert': 'write',
  'targets.verify': 'write',
  'art.url': 'read',
  'art.rerender': 'write',
  'identity.list': 'read',
  'identity.confirm': 'write',
  'stats.reveal': 'read',
  'collections.list': 'read',
  'collections.upsert': 'write',
  'collections.remove': 'write',
  'session.stop': 'write',
  'session.focus': 'write',
  // [p2] §20.8. Both reads answer out of the index. `accounts.cancelConnect` is idempotent on
  // the wire and `write` here on purpose: it ends live core-side state, and this table is
  // deliberately the wider of the two — erring toward `write` costs a refusal, erring toward
  // `read` acts twice.
  'accounts.list': 'read',
  'accounts.orgs': 'read',
  'accounts.connect': 'write',
  'accounts.cancelConnect': 'write',
  'accounts.connectPat': 'write',
  'accounts.upgradeScope': 'write',
  'accounts.disconnect': 'write',
  'accounts.setOrgEnabled': 'write',
  // Reads one stored key and the account hosts, writes nothing and spawns nothing.
  'remote.webUrl': 'read',
  // [p2] §25.5. Opens one file under a location root; no write, no process, no socket.
  'projects.readme': 'read',
  // [p2] §25.5. Writes the consent column, and the schema calls it non-idempotent for the same
  // reason: a consent replayed through a core restart re-grants a decision the user made once.
  'projects.setReadmeRemote': 'write',
  // [p2] §25.5. Reads bytes and issues requests; it writes nothing and spawns nothing, and a
  // replayed read costs at worst the same images again.
  'projects.readmeAssets': 'read',
  // [p2] §24.9. The preview composes a destination and compares it against what is on disk; it
  // spawns nothing and writes nothing. The other two spawn git and remove a staging directory,
  // and a replay of either acts twice on a filesystem — which is the whole of why this table is
  // wider than the schema's `idempotent` flag.
  'install.preview': 'read',
  'install.start': 'write',
  'install.cancel': 'write',
  // [p2] §21.13. A read of two tables and the runner's own process state. It queues nothing and
  // spawns nothing, so replaying it costs a second answer to the same question.
  'sync.status': 'read',
  // [p3] §33.8. It deserialises one stored scene document and answers geometry — no table is
  // written, no job enqueued, no process spawned.
  'health.weathering': 'read',
};

export function isNonIdempotent(name: CommandName): boolean {
  return COMMAND_EFFECT[name] === 'write';
}

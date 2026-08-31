/**
 * The only file in the palette-and-collections work that names a *value* another plan owns.
 * If plan 13 or plan 12 spells one of these differently, change it here and nowhere else —
 * every other module takes a `QueryEngine`, a `CoreRpc` or a component as a parameter.
 * R13: the AST type itself is plan 13's, so nothing below casts through `never`.
 *
 * `AttentionChip` / `AttentionChipProps` are re-exported below: §8.0b's box now exists as a
 * component and the collection chip composes it through this file and no other.
 *
 * **One re-export this file is supposed to carry is still absent, and is not written here.**
 * `jewelFor` (`app/src/renderer/derive/jewel.ts`, plan 12) does not exist in the tree yet. A
 * local substitute would be the second copy this file exists to prevent, so it is named as a
 * gap instead: when plan 12 lands it, it is re-exported from here and from nowhere else.
 */
// R43: plan 13's own names, at plan 13's own paths. There is no `./query/index.js`, and nothing
// below is aliased back to the name this plan first guessed at — an alias is a second name for
// one thing, which is the defect R12 and R43 both exist to remove.
import type { CommandArgs, CommandName, CommandResult } from '../generated/protocol.js';
import { queryHasField, queryTermCount } from '../shared/query/ast.js';
import { effectiveQueryText } from '../shared/query/format.js';
import { QUERY_GRAMMAR_VERSION } from '../shared/query/grammar.js';
import { parseQuery } from '../shared/query/parse.js';
import type { CoreRpc, QueryEngine } from './collections/engine.js';
import type { QueryContext } from './shelf/evaluate.js';
import { evaluateQuery } from './shelf/evaluate.js';
import { toShelfRow } from './shelf/row.js';

// §8.0b's chip box, plan 13c's component. The collection chip names it here and nowhere else.
export { AttentionChip, DEFAULT_CHIP_ACCENT } from './shelf/AttentionChip.js';
export type { AttentionChipProps } from './shelf/AttentionChip.js';

/**
 * R43: an engine per `QueryContext`, not a module constant. `evaluateQuery` is three-valued and
 * needs the context to know what the projection can answer at all; §8.8's counts are only "the
 * same function the shelf runs" if they run against the same context. The caller holds one
 * engine for the current context (`useMemo` over it) and passes it down.
 */
export function createRendererEngine(ctx: QueryContext): QueryEngine {
  return {
    grammarVersion: QUERY_GRAMMAR_VERSION,
    /**
     * R13: the AST is plan 13's own type, so §8.3a's soft errors are read off it — `ast.ignored` —
     * rather than guessed at. Nothing here re-tokenizes to find them.
     */
    parse: (text) => {
      const ast = parseQuery(text);
      return { ast, dropped: ast.ignored.map((t) => ({ text: t.text, reason: t.reason })) };
    },
    canonical: (ast) => effectiveQueryText(ast),
    /**
     * `evaluateQuery` takes `(rows, ast, ctx)` and returns `{ rows, ignored }`; the seam wants
     * `(ast, rows)` and the matched rows. Adapting the shape is this file's whole job.
     * `ShelfRow` is `ProjectRow & ProjectRowExtras`, so the result is a `ProjectRow[]` already,
     * and `toShelfRow` is idempotent — it fills the extras with `null` when the wire row does
     * not carry them. The dropped terms `evaluateQuery` also returns are §8.3a's *not
     * answerable* set; `collectionQueryHealth` reads its dropped terms off `parse`, so nothing
     * here needs them.
     */
    filter: (ast, rows) => evaluateQuery(rows.map(toShelfRow), ast, ctx).rows,
    termCount: (ast) => queryTermCount(ast),
    hasField: (ast, field) => queryHasField(ast, field),
  };
}

interface BridgeReplyLike {
  readonly ok: boolean;
  readonly value?: unknown;
  readonly error?: { readonly code: string; readonly message: string };
}

interface RequestBridge {
  readonly request: (name: string, args: unknown) => Promise<BridgeReplyLike>;
}

export const rendererRpc: CoreRpc = {
  async request<K extends CommandName>(name: K, args: CommandArgs[K]): Promise<CommandResult[K]> {
    const bridge = (globalThis as { codotheca?: RequestBridge }).codotheca;
    if (bridge === undefined) throw new Error('the core bridge is not available');
    const reply = await bridge.request(name, args);
    if (!reply.ok) {
      // §2.4: the core's `message` is diagnostic and is never shown raw. The caller decides
      // what the user reads; this only carries the code far enough to be logged.
      throw new Error(reply.error?.code ?? 'INTERNAL');
    }
    return reply.value as CommandResult[K];
  },
};

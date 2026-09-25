import { describe, expect, it, vi } from 'vitest';
import type { RootAdd, RootId, RootSuggestion } from '../generated/protocol';
import type { CommitSuggestionReply } from '../shared/channels';
import { registerSuggestionCommit, SuggestionCache } from './rootPicker';

/**
 * GAP-16b-1. On first run, ticking only *suggested* roots and pressing `DIG IN` added **no
 * root**: `commitSuggestion` resolved against a cache that existed in no file.
 *
 * §2.4 forbids the renderer from originating a filesystem path, which is the whole reason this
 * is awkward — the renderer holds only the display string. The shell resolves it against what
 * `roots.suggest` returned and calls `roots.add` with the folder the core itself proposed.
 */
function suggestion(pathDisplay: string): RootSuggestion {
  return {
    pathDisplay,
    kind: 'linux',
    distro: '',
    provenance: 'convention',
    provenanceDetail: null,
    hits: null,
    preTicked: true,
  };
}

const ADDED: RootAdd = {
  root: {
    id: 1 as RootId,
    pathDisplay: '/home/u/code',
    kind: 'linux',
    distro: '',
    enabled: true,
    descendIntoRepos: false,
    provenance: 'convention',
    state: 'watched',
    addedAt: 1,
    projectCount: null,
  },
  refusedBecause: null,
  estimatedDirs: null,
};

interface Harness {
  readonly invoke: (payload: unknown) => Promise<CommitSuggestionReply>;
  readonly addRoot: ReturnType<typeof vi.fn>;
  readonly suggestRoots: ReturnType<typeof vi.fn>;
}

function harness(rows: readonly RootSuggestion[]): Harness {
  const addRoot = vi.fn(() => Promise.resolve(ADDED));
  const suggestRoots = vi.fn(() => Promise.resolve(rows));
  let handler = null as ((payload: unknown) => Promise<CommitSuggestionReply>) | null;
  registerSuggestionCommit({
    suggestRoots,
    addRoot,
    cache: new SuggestionCache(),
    handle: (_channel, fn) => {
      handler = fn;
    },
  });
  const bound = handler;
  if (bound === null) throw new Error('the commit channel was never registered');
  return { invoke: bound, addRoot, suggestRoots };
}

describe('committing a suggested root', () => {
  it('adds the folder the core proposed', async () => {
    const h = harness([suggestion('/home/u/code')]);
    const reply = await h.invoke({ pathDisplay: '/home/u/code' });

    expect(reply.kind).toBe('added');
    expect(h.addRoot).toHaveBeenCalledTimes(1);
    // §10.1b: the directory estimate is taken only on a folder the user picked in a dialog,
    // never on a row the core proposed — the core's own cache says the same from its side.
    expect(h.addRoot.mock.calls[0]?.[0]).toMatchObject({ confirmLarge: true });
  });

  it('refuses a display string that names no suggestion, and adds nothing', async () => {
    const h = harness([suggestion('/home/u/code')]);
    const reply = await h.invoke({ pathDisplay: '/home/u/elsewhere' });

    expect(reply).toEqual({ kind: 'unknown' });
    expect(h.addRoot).not.toHaveBeenCalled();
  });

  it('refuses rather than picking one when two suggestions render the same string', async () => {
    // Two conventional locations can produce one display string. Adding the wrong folder is
    // worse than adding none, so this is a refusal and never a first match.
    const h = harness([suggestion('/home/u/code'), suggestion('/home/u/code')]);
    const reply = await h.invoke({ pathDisplay: '/home/u/code' });

    expect(reply).toEqual({ kind: 'ambiguous' });
    expect(h.addRoot).not.toHaveBeenCalled();
  });

  it('re-reads the suggestions on every commit, so a folder that has gone is refused', async () => {
    const h = harness([suggestion('/home/u/code')]);
    await h.invoke({ pathDisplay: '/home/u/code' });
    await h.invoke({ pathDisplay: '/home/u/code' });
    expect(h.suggestRoots).toHaveBeenCalledTimes(2);
  });

  it('refuses a malformed payload without asking the core anything', async () => {
    const h = harness([suggestion('/home/u/code')]);
    const reply = await h.invoke({ pathDisplay: 7 });

    expect(reply.kind).toBe('failed');
    expect(h.suggestRoots).not.toHaveBeenCalled();
    expect(h.addRoot).not.toHaveBeenCalled();
  });
});

describe('the suggestion cache', () => {
  it('answers only for a string exactly one suggestion carries', () => {
    const cache = new SuggestionCache();
    cache.remember([suggestion('/home/u/code'), suggestion('/home/u/work')]);
    expect(cache.resolve('/home/u/code')).toBe('/home/u/code');
    expect(cache.resolve('/home/u/nothing')).toBeNull();

    cache.remember([suggestion('/home/u/code'), suggestion('/home/u/code')]);
    expect(cache.resolve('/home/u/code')).toBeNull();
    expect(cache.count('/home/u/code')).toBe(2);
  });
});

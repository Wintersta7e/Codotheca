import { cleanup, render, screen, waitFor } from '@testing-library/react';
import type { ReactElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Collection, CollectionId, ProjectId } from '../../generated/protocol.js';
import type { QueryAst, QueryTerm } from '../../shared/query/ast.js';
import { statesAColour } from '../a11y/names.js';
import { AttentionRow } from '../shelf/AttentionRow.js';
import type { ShelfCounts } from '../shelf/counts.js';
import type { QueryContext } from '../shelf/evaluate.js';
import type { ShelfRow } from '../shelf/row.js';
import { makeProjectRow } from '../testing/projectRow.js';
import { CollectionChip, removeControlName } from './CollectionChip.js';
import { CollectionChipRow } from './CollectionChipRow.js';
import { ARM_SUB_LINE } from './armedDelete.js';
import { collectionChipModel } from './chipModel.js';
import { fakeEngine } from './engine.js';
import { CollectionNameInput, SaveQueryButton, useSaveQueryFlow } from './SaveQuery.js';
import { CollectionsProvider, type CollectionsApi } from './store.js';
import css from './collections.css?raw';

afterEach(cleanup);

/**
 * This file is the *rendered* half of the acceptance surface. The source-text bans — `FORGET`,
 * the roast, `--text-4`/`--text-5`, `view_state` — are scanned over the whole directory by
 * `app/test/collectionsBans.test.ts`, which runs in the node project because `node:fs` has no
 * types under `tsconfig.web.json` and a directory walk beats a hand-written list of imports.
 */

const saved = (over: Partial<Collection> = {}): Collection => ({
  id: 7 as CollectionId,
  name: 'Rust work',
  kind: 'query',
  queryText: 'lang:rust is:dirty',
  queryGrammarVersion: 1,
  sortIndex: 0,
  memberCount: null,
  ...over,
});

const rows = [makeProjectRow({ id: 1 as ProjectId })];
const engine = fakeEngine({ filter: (_a, all) => all });

const ast = (...texts: readonly string[]): QueryAst => ({
  grammarVersion: 1,
  terms: texts.map((text): QueryTerm => ({ kind: 'bare', negated: false, text })),
  ignored: [],
});

const textOf = (term: QueryTerm): string => (term.kind === 'bare' ? term.text : '');

const brokenEngine = fakeEngine({ grammarVersion: 1 });
const degradedEngine = fakeEngine({
  parse: () => ({ ast: ast('lang:rust'), dropped: [{ text: 'x:1', reason: 'unknownField' }] }),
  canonical: () => 'lang:rust',
  filter: (_a, all) => all,
});

describe('the bans', () => {
  it('names nothing by its colour', () => {
    for (const state of ['normal', 'degraded', 'broken'] as const) {
      const collection = state === 'broken' ? saved({ queryGrammarVersion: 9 }) : saved();
      const e = state === 'broken' ? brokenEngine : state === 'degraded' ? degradedEngine : engine;
      const model = collectionChipModel(collection, rows, e);
      expect(statesAColour(model.accessibleName)).toBe(false);
    }
    expect(statesAColour(removeControlName('Rust work', true))).toBe(false);
    expect(statesAColour(removeControlName('Rust work', false))).toBe(false);
  });
});

/**
 * R36's shape, one level down: a stylesheet class that no component applies is a rule that
 * selects nothing, and no gate says so. Every class this directory's stylesheet declares must be
 * on an element one of its components really renders.
 */
describe('every class collections.css declares is applied by something', () => {
  it('matches nodes the components actually render', async () => {
    // Vacuity first: outside `vitest.config.ts`'s css include this `?raw` import is the empty
    // string, and every class below would be trivially "applied".
    expect(css.length).toBeGreaterThan(400);
    const declared = new Set([...css.matchAll(/\.(cdt-[a-z0-9-]+)/g)].map((m) => m[1] ?? ''));
    expect(declared.size).toBeGreaterThan(5);

    function Naming(): ReactElement {
      const flow = useSaveQueryFlow({ queryText: 'lang:rust', engine, onRestoreQuery: vi.fn() });
      return (
        <>
          <button type="button" onClick={flow.begin}>
            begin
          </button>
          <CollectionNameInput flow={flow} />
          <SaveQueryButton flow={flow} />
        </>
      );
    }

    const api: CollectionsApi = {
      list: () => Promise.resolve([]),
      create: () => Promise.resolve(null),
      remove: () => Promise.resolve(),
    };
    const { container } = render(
      <CollectionsProvider api={api}>
        {/* Armed replaces the sub-line with the confirm wording, so the struck terms only
            appear on a chip that is not armed. Both are needed to reach every class. */}
        <CollectionChip
          model={collectionChipModel(saved(), rows, degradedEngine)}
          active={false}
          armed
          onActivate={vi.fn()}
          onPressRemove={vi.fn()}
          onDisarm={vi.fn()}
        />
        <CollectionChip
          model={collectionChipModel(saved(), rows, degradedEngine)}
          active={false}
          armed={false}
          onActivate={vi.fn()}
          onPressRemove={vi.fn()}
          onDisarm={vi.fn()}
        />
        <Naming />
      </CollectionsProvider>,
    );
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'begin' })).toBeTruthy();
    });
    screen.getByRole('button', { name: 'begin' }).click();
    await waitFor(() => {
      expect(screen.queryByRole('textbox', { name: 'Collection name' })).not.toBeNull();
    });

    const applied = new Set<string>();
    for (const element of container.querySelectorAll('*')) {
      for (const name of element.classList) applied.add(name);
    }
    expect([...declared].filter((name) => !applied.has(name))).toEqual([]);
  });
});

describe('criterion 50 — the states are legible with no colour and no motion', () => {
  it('arm, degrade and break each carry their own wording', () => {
    expect(ARM_SUB_LINE).toContain('THE PROJECTS STAY');
    const degraded = collectionChipModel(saved(), rows, degradedEngine);
    expect(degraded.accessibleName).toContain('no longer parses');
    const broken = collectionChipModel(saved({ queryGrammarVersion: 9 }), rows, brokenEngine);
    expect(broken.accessibleName).toContain('not counted');
  });
});

describe('criterion 9 — a saved query survives a restart', () => {
  it('re-lights from collections.list and the restored query, with no view_state key', async () => {
    const ordered = fakeEngine({
      filter: (_a, all) => all,
      canonical: (a) => a.terms.map(textOf).sort().join(' '),
    });
    render(
      <CollectionsProvider
        api={{
          list: () => Promise.resolve([saved()]),
          create: () => Promise.resolve(null),
          remove: () => Promise.resolve(),
        }}
      >
        <CollectionChipRow
          rows={rows}
          currentQuery="is:dirty lang:rust"
          engine={ordered}
          nowMs={() => 0}
          onQueryChange={vi.fn()}
        />
      </CollectionsProvider>,
    );
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^Rust work/ }).getAttribute('aria-pressed')).toBe(
        'true',
      );
    });
    // The negative half — that nothing here mirrors a collection into `view_state` — is scanned
    // over the whole directory by `app/test/collectionsBans.test.ts`.
  });
});

/**
 * The mount contract, executable rather than written down. §8.8's saved chips are appended
 * **after** the four built-ins, inside plan 13c's own wrapping row — this is the assertion a
 * neighbour reads instead of a paragraph.
 */
describe('the mount contract for AttentionRow', () => {
  const ctx = {
    now: 0,
    firstRunCompletedAt: null,
    collectionIdsByName: new Map(),
    pathsAreCaseSensitive: false,
    commitSubjectHits: null,
    capabilities: {
      authoredByUser: false,
      location: false,
      hasReadme: false,
      hasLicense: false,
      hasTests: false,
      hasCi: false,
      hasRemote: false,
      hasSubmodules: false,
    },
  } as unknown as QueryContext;
  const counts: ShelfCounts = {
    matched: 1,
    total: 1,
    reference: 0,
    classified: 0,
    classificationKnown: false,
  };
  const shelfRows = [{ id: 1, ahead: 2, isReference: false }] as unknown as ShelfRow[];

  it('appends the saved chips after ALL, UNPUSHED, UNCOMMITTED and COLD', async () => {
    render(
      <CollectionsProvider
        api={{
          list: () => Promise.resolve([saved({ name: 'Rust work', sortIndex: 0 })]),
          create: () => Promise.resolve(null),
          remove: () => Promise.resolve(),
        }}
      >
        <AttentionRow
          rows={shelfRows}
          ctx={ctx}
          counts={counts}
          sortLabel="LAST TOUCHED"
          query=""
          onQuery={vi.fn()}
          savedChips={
            <CollectionChipRow
              rows={rows}
              currentQuery=""
              engine={engine}
              nowMs={() => 0}
              onQueryChange={vi.fn()}
            />
          }
        />
      </CollectionsProvider>,
    );
    await waitFor(() => {
      expect(document.querySelectorAll('.cdt-attention-label')).toHaveLength(5);
    });
    const labels = [...document.querySelectorAll('.cdt-attention-label')].map(
      (element) => element.textContent,
    );
    expect(labels).toEqual(['ALL', 'UNPUSHED', 'UNCOMMITTED', 'COLD', 'RUST WORK']);
  });

  // Whatever the saved tail decides to render, the four built-ins are unaffected by it.
  it('renders the four built-ins alone when the tail renders nothing', () => {
    render(
      <AttentionRow
        rows={shelfRows}
        ctx={ctx}
        counts={counts}
        sortLabel="LAST TOUCHED"
        query=""
        onQuery={vi.fn()}
        savedChips={null}
      />,
    );
    const labels = [...document.querySelectorAll('.cdt-attention-label')].map(
      (element) => element.textContent,
    );
    expect(labels).toEqual(['ALL', 'UNPUSHED', 'UNCOMMITTED', 'COLD']);
  });
});

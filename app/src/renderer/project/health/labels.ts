/**
 * [p3] The words a person reads for each debt source and each decay layer — **one owner** for
 * the `HEALTH` tab's check rows, its debt-item list and the settings drawer's check switches.
 * The raw `DebtSource` and `DecayLayer` ids stay in `data-*` attributes and never reach the page.
 *
 * **A source has two names, because a check and its item say opposite things.** The check row
 * and the drawer switch name what is looked at, which is true of every project whatever it finds:
 * `README · PASSED`, never `No README · PASSED`. An item is the problem itself, so it keeps the
 * problem's words: `No README`. Where §28.2 pairs a source with a §31 check, the check takes that
 * check's name, since the checklist on the same tab already calls the same fact by it.
 *
 * `Record<…>` over the generated unions makes a missing label a type error, and `labels.test.ts`
 * checks the same against the schema. A newer core can still send a variant this build does not
 * know; it reads as its id, because a blank name hides a fact the id at least states.
 */
import type { DebtSource, DecayLayer } from '../../../generated/protocol';

export interface SourceLabel {
  /** What the check looks at — the check row and the drawer switch. */
  readonly check: string;
  /** The problem one item is — the debt-item row. */
  readonly item: string;
}

export const SOURCE_LABELS: Readonly<Record<DebtSource, SourceLabel>> = {
  todo_marker: { check: 'TODO, FIXME and HACK markers', item: 'TODO, FIXME or HACK marker' },
  missing_readme: { check: 'README', item: 'No README' },
  missing_license: { check: 'LICENSE', item: 'No LICENSE' },
  missing_tests: { check: 'Tests', item: 'No tests' },
  no_release: { check: 'Release', item: 'No tagged release' },
  unpushed_commits: { check: 'Pushed', item: 'Unpushed commits' },
  ci_red: { check: 'CI green', item: 'CI is red' },
  dependency_advisory: {
    check: 'Dependency advisories',
    item: 'Dependency with a published advisory',
  },
  abandoned_with_debt: { check: 'Abandonment with open debt', item: 'Abandoned with open debt' },
};

/** §33's five layers, as the list heads its groups. */
export const LAYER_LABELS: Readonly<Record<DecayLayer, string>> = {
  dust: 'DUST',
  cobwebs: 'COBWEBS',
  rust: 'RUST',
  cracks: 'CRACKS',
  overgrowth: 'OVERGROWTH',
};

function known<T>(table: Readonly<Record<string, T>>, key: string): T | undefined {
  return Object.hasOwn(table, key) ? table[key] : undefined;
}

export function checkLabel(source: DebtSource): string {
  return known(SOURCE_LABELS, source)?.check ?? source;
}

export function itemLabel(source: DebtSource): string {
  return known(SOURCE_LABELS, source)?.item ?? source;
}

/** Upper-cased, as the layer heads read before this table existed. */
export function layerLabel(layer: DecayLayer): string {
  return known(LAYER_LABELS, layer) ?? layer.toUpperCase();
}

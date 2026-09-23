/**
 * [p3] The words a person reads for each debt source and each decay layer — **one owner** for
 * the `HEALTH` tab's check rows, its debt-item list and the settings drawer's check switches.
 * The raw `DebtSource` and `DecayLayer` ids stay in `data-*` attributes and never reach the page.
 *
 * §28 and §30 name the sources by id only, so the wording comes from the design's own debt rows
 * and `concept.md`'s source list. A source is named for what it finds, because a check is a debt
 * source (§30.3): `No README · OPEN` is the item, and `No README · PASSED` is the check that looked.
 *
 * `Record<…>` over the generated unions makes a missing label a type error, and `labels.test.ts`
 * checks the same against the schema, so a tenth source cannot ship unlabelled.
 */
import type { DebtSource, DecayLayer } from '../../../generated/protocol';

export const SOURCE_LABELS: Readonly<Record<DebtSource, string>> = {
  todo_marker: 'TODO, FIXME and HACK markers',
  missing_readme: 'No README',
  missing_license: 'No LICENSE',
  missing_tests: 'No tests',
  no_release: 'No tagged release',
  unpushed_commits: 'Unpushed commits',
  ci_red: 'CI is red',
  dependency_advisory: 'Dependencies with published advisories',
  abandoned_with_debt: 'Abandoned with open debt',
};

/** §33's five layers, as the list heads its groups. */
export const LAYER_LABELS: Readonly<Record<DecayLayer, string>> = {
  dust: 'DUST',
  cobwebs: 'COBWEBS',
  rust: 'RUST',
  cracks: 'CRACKS',
  overgrowth: 'OVERGROWTH',
};

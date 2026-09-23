/**
 * [p3] §33.2 — **every open item on the page, in text, unconditionally.** The layer set is never
 * the only rendering of an item: a layer with no anchor draws nothing, and this list is what keeps
 * that an ornament lost rather than a fact lost.
 *
 * **Grouped by `DebtItem.layer`, in the renderer (R120).** `groupByLayer` leaves a layer with no
 * items out of the map, so an empty layer draws no group — absent, never empty.
 *
 * **Every item renders; not every item counts.** A `shown_only` item is visible and is excluded
 * from the reading's `scoredOpen` (A7); an `unverified` one is carried so the list can say *this
 * item is not being counted right now*, and is counted by nothing (R128/F10). **No count renders
 * here** — the reading's pair above is the only figure on this tab.
 *
 * The list renders what it is handed. A check switched off has its items removed by the core
 * (§30.9), so nothing here filters by switch, and nothing here puts them back.
 */
import { Fragment, type ReactElement } from 'react';

import type { AdvisoryDetail, DebtItem, DecayLayer } from '../../../generated/protocol';
import { DECAY_LAYER_ORDER } from '../../decay/layers';
import { groupByLayer } from '../../debt/groupByLayer';
import { PART_SEPARATOR } from './checkForms';

export const DEBT_LIST_TITLE = 'DEBT ITEMS';
export const UNVERIFIED_NOTE = 'not being counted right now — its evidence cannot be seen';
export const SHOWN_ONLY_NOTE = 'shown, not counted';

/** Declaration order, and a layer this build does not know sorts last rather than vanishing. */
function layerRank(layer: DecayLayer): number {
  const index = DECAY_LAYER_ORDER.indexOf(layer);
  return index < 0 ? DECAY_LAYER_ORDER.length : index;
}

function whereOf(item: DebtItem): string | null {
  if (item.pathDisplay === null) return null;
  const line = item.line === null ? '' : `:${String(item.line)}`;
  const column = item.line === null || item.column === null ? '' : `:${String(item.column)}`;
  return `${item.pathDisplay}${line}${column}`;
}

/**
 * §32's display attributes, as the source spelled them. **`severity` is rendered verbatim** —
 * never mapped, never re-cased (§32.13) — and **every** CVE id renders; an advisory with none is
 * named by its advisory id alone.
 */
function advisoryParts(advisory: AdvisoryDetail): string[] {
  const parts = [advisory.ecosystem, advisory.packageName, advisory.advisoryId];
  parts.push(...advisory.cveIds);
  if (advisory.severity !== null) parts.push(`severity ${advisory.severity}`);
  if (advisory.fixedVersion !== null) parts.push(`fixed in ${advisory.fixedVersion}`);
  return parts;
}

function noteOf(item: DebtItem): string | null {
  if (item.state === 'unverified') return UNVERIFIED_NOTE;
  if (item.scoring === 'shown_only') return SHOWN_ONLY_NOTE;
  return null;
}

interface Part {
  readonly kind: 'source' | 'where' | 'text' | 'advisory' | 'fingerprint' | 'note';
  readonly text: string;
}

function partsOf(item: DebtItem): Part[] {
  const parts: Part[] = [{ kind: 'source', text: item.source }];
  const where = whereOf(item);
  if (where !== null) parts.push({ kind: 'where', text: where });
  if (item.salientText !== null) parts.push({ kind: 'text', text: item.salientText });
  if (item.advisory !== null) {
    for (const text of advisoryParts(item.advisory)) parts.push({ kind: 'advisory', text });
  }
  // The fingerprint names an item only when nothing else does: a content item's is a hash, and
  // its path and text say more; an advisory's is `<ecosystem>:<package>:<id>`, readable as is.
  if (parts.length === 1 && item.fingerprint !== '') {
    parts.push({ kind: 'fingerprint', text: item.fingerprint });
  }
  const note = noteOf(item);
  if (note !== null) parts.push({ kind: 'note', text: note });
  return parts;
}

function DebtRow({ item }: { readonly item: DebtItem }): ReactElement {
  return (
    <li
      className="cp-health-debt-item"
      data-source={item.source}
      data-state={item.state}
      data-scoring={item.scoring}
    >
      {/* Separated in the text itself, so the row reads as parts before any stylesheet does. */}
      {partsOf(item).map((part, index) => (
        <Fragment key={index}>
          {index === 0 ? null : PART_SEPARATOR}
          <span className={`cp-health-debt-${part.kind}`}>{part.text}</span>
        </Fragment>
      ))}
    </li>
  );
}

export interface DebtListProps {
  readonly debt: readonly DebtItem[];
}

export function DebtList({ debt }: DebtListProps): ReactElement | null {
  // An empty list says nothing: *nothing open* is the reading's to claim, under §30.4's gate,
  // and a line here would claim it without the basis that makes it honest.
  if (debt.length === 0) return null;
  const groups = [...groupByLayer(debt)].sort(([a], [b]) => layerRank(a) - layerRank(b));
  return (
    <section className="cp-health-debt" data-testid="cp-health-debt" aria-label="Debt items">
      <h3 className="cp-health-debt-title">{DEBT_LIST_TITLE}</h3>
      {groups.map(([layer, items]) => (
        <section key={layer} className="cp-health-debt-layer" data-layer={layer}>
          <h4 className="cp-health-debt-layer-name">{layer.toUpperCase()}</h4>
          <ul className="cp-health-debt-items">
            {items.map((item) => (
              <DebtRow key={`${item.source}:${item.fingerprint}`} item={item} />
            ))}
          </ul>
        </section>
      ))}
    </section>
  );
}

import type { KeyboardEvent, ReactElement, ReactNode } from 'react';
import type { CollectionId } from '../../generated/protocol.js';
import { AttentionChip } from '../upstream.js';
import { ARM_SUB_LINE } from './armedDelete.js';
import {
  COLLECTION_ACCENT,
  type CollectionChipModel,
  type CollectionSubLine,
} from './chipModel.js';
import './collections.css';

export const REMOVE_GLYPH = '×';

/**
 * §8.8 names the chip; it does not name the `×`. Composed here on §8.8's own rule — named by
 * what it removes, never by its shape or its colour — and saying out loud what the confirm line
 * says. `collections.remove` deletes the `collection` row and its `collection_member` rows and
 * touches nothing else: no project, no location, no session, no byte on disk (§17).
 */
export function removeControlName(name: string, armed: boolean): string {
  const base = `Remove the collection ${name}`;
  return armed ? `${base}. Press again to confirm. The projects stay.` : base;
}

function subLineNode(subLine: CollectionSubLine, armed: boolean): ReactNode {
  if (armed) return ARM_SUB_LINE;
  if (subLine.kind === 'text') return subLine.text;
  // §8.3a's soft-error rendering, in the sub-line: the survivors plain, the dropped struck.
  return (
    <>
      <span>{subLine.ran}</span>
      {subLine.dropped.map((term) => (
        <span key={term} className="cdt-collection-dropped">
          {term}
        </span>
      ))}
    </>
  );
}

export interface CollectionChipProps {
  readonly model: CollectionChipModel;
  readonly active: boolean;
  readonly armed: boolean;
  /** §8.8: activating writes the collection's query into the query field. */
  readonly onActivate: (query: string) => void;
  readonly onPressRemove: (id: CollectionId) => void;
  readonly onDisarm: () => void;
}

/**
 * §8.0b's box with four additions and no restatement. `AttentionChip` supplies the number, the
 * label, the ground, the accent edge and the hover; this supplies the sub-line's two shapes, the
 * broken outline, the armed pin and the `×`.
 *
 * The `×` is a **sibling** button, never a nested one: §8.8 requires the chip itself to be a real
 * `<button>` with `aria-pressed`, and a button inside a button is not focusable, not announced
 * and not valid. Both sit inside `.cdt-collection`, which carries the state attributes and the
 * `keydown` handler, so `Delete` on a focused chip reaches the same call as the `×`.
 */
export function CollectionChip(props: CollectionChipProps): ReactElement {
  const { model, active, armed } = props;

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>): void => {
    if (event.key === 'Delete') {
      event.preventDefault();
      props.onPressRemove(model.id);
      return;
    }
    // The shelf binds Esc to closing Peek (§11.7). While armed, this press is spent here; while
    // nothing is armed the key is the shelf's and is not swallowed.
    if (event.key === 'Escape' && armed) {
      event.preventDefault();
      event.stopPropagation();
      props.onDisarm();
    }
  };

  return (
    <div
      className="cdt-collection"
      data-state={model.state}
      data-armed={armed ? 'true' : 'false'}
      data-active={active ? 'true' : 'false'}
      onKeyDown={onKeyDown}
    >
      <AttentionChip
        accent={COLLECTION_ACCENT}
        /* `null` renders no element at all — §8.8: broken carries no number, and a blank is the
           only reading that is not a claim. */
        count={model.count}
        label={model.label}
        subLine={
          <span className="cdt-collection-subline">{subLineNode(model.subLine, armed)}</span>
        }
        active={active}
        disabled={!model.activatable}
        accessibleName={model.accessibleName}
        onActivate={() => {
          if (model.activationQuery !== null) props.onActivate(model.activationQuery);
        }}
      />
      <button
        type="button"
        className="cdt-collection-remove"
        aria-label={removeControlName(model.name, armed)}
        onClick={() => {
          props.onPressRemove(model.id);
        }}
      >
        {REMOVE_GLYPH}
      </button>
    </div>
  );
}

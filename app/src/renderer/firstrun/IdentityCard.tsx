/**
 * §1.4's card, inside §8.0's box at priority 4a.
 *
 * The box, its chrome and its dismissal machinery are the notice slot's. This file owns what is
 * inside: a disclosure, one row per seeded address, the statement of effect, and two actions.
 * Every string is `copy.ts`'s — R12, and this card's copy is the one place the product tells the
 * user what a tick already did.
 */
import { useState } from 'react';
import type { ReactElement } from 'react';
import type { IdentityConfirm, IdentityRow } from '../../generated/protocol';
import {
  IDENTITY_BODY_1,
  IDENTITY_BODY_2,
  IDENTITY_CONFIRM_LABEL,
  IDENTITY_FOOTNOTE,
  IDENTITY_LEAVE_LABEL,
  IDENTITY_NO_COMMITS,
  IDENTITY_TITLE,
} from './copy';

function plural(n: number, one: string, many: string): string {
  return `${n.toLocaleString()} ${n === 1 ? one : many}`;
}

/** §1.4's provenance table. Why this address is in the set — the line that decides a tick. */
function provenance(row: IdentityRow): string {
  if (row.source === 'noreply') return 'GITHUB NOREPLY ADDRESS';
  if (row.source === 'manual') return 'ADDED BY YOU';
  if (row.source === 'gitconfig') {
    return row.repositories === null
      ? 'GIT CONFIG · GLOBAL'
      : `GIT CONFIG · ${plural(row.repositories, 'REPOSITORY', 'REPOSITORIES')}`;
  }
  if (row.aliasReason === 'local_part' && row.primaryEmail !== null) {
    return `SAME LOCAL PART AS ${row.primaryEmail}`;
  }
  if (row.aliasReason === 'coauthor') {
    return `CO-AUTHORED WITH YOU IN ${plural(row.repositories ?? 0, 'REPOSITORY', 'REPOSITORIES')}`;
  }
  // §1.4's table has no row for an inferred address with no reason recorded; naming the
  // mechanism is still truthful and still decides a tick.
  return 'INFERRED FROM AN ADDRESS OF YOURS';
}

/**
 * The row's second line: why it was seeded, and what unticking it would cost.
 *
 * §1.4 appends the weight after ` · ` because "without it the row is unjudgeable — an address
 * with four commits and one with nine thousand look identical, and the entire cost of unticking
 * is which projects fall out". An address that has authored nothing here is **worded**: a zero
 * standing beside counts in the thousands reads as a failed lookup, not as a real zero.
 */
export function identitySourceLine(row: IdentityRow): string {
  const weight =
    row.commits === 0
      ? IDENTITY_NO_COMMITS
      : `${plural(row.commits, 'COMMIT', 'COMMITS')} IN ${plural(row.projects, 'PROJECT', 'PROJECTS')}`;
  return `${provenance(row)} · ${weight}`;
}

/**
 * §1.4's statement of effect, from the `apply:false` preview and never from the write.
 *
 * The same string stands before and after the write: the preview *is* the join the write
 * performs, on a single-user local database where nothing can intervene between them. Swapping
 * in a past-tense receipt would imply a second computation had happened.
 *
 * Null where the set is unchanged — the action then confirms the set as seeded, and there is no
 * effect to state.
 */
export function identityEffectLine(
  preview: IdentityConfirm | null,
  untickedCount: number,
): string | null {
  if (preview === null || untickedCount === 0) return null;
  if (preview.movedToReference === 0) {
    const subject = untickedCount === 1 ? 'that address' : 'those addresses';
    return `Nothing moves — no commits in this library are from ${subject}.`;
  }
  return `Recomputing — ${plural(preview.movedToReference, 'project', 'projects')} moved to Reference.`;
}

export interface IdentityCardProps {
  readonly rows: readonly IdentityRow[];
  /** The reply to `identity.confirm {apply:false}` for the current ticks. Null until it lands. */
  readonly preview: IdentityConfirm | null;
  /** §1.4: the effect precedes the write, so every tick change asks for a fresh preview. */
  readonly onPreview: (emails: readonly string[]) => void;
  readonly onConfirm: (emails: readonly string[]) => void;
  /** §8.0: unscoped and one-shot. Writes `notice.dismissed.identity` and nothing else. */
  readonly onLeaveAsIs: () => void;
}

export function IdentityCard(props: IdentityCardProps): ReactElement {
  // §1.4: every seeded row arrives ticked, because the seeded set is the set already in force.
  const [unticked, setUnticked] = useState<ReadonlySet<string>>(new Set());
  const ticked = props.rows.map((row) => row.email).filter((email) => !unticked.has(email));
  const effect = identityEffectLine(props.preview, unticked.size);

  const toggle = (email: string): void => {
    const next = new Set(unticked);
    if (!next.delete(email)) next.add(email);
    setUnticked(next);
    props.onPreview(props.rows.map((row) => row.email).filter((each) => !next.has(each)));
  };

  return (
    <div className="cdt-fr-identity">
      <p className="cdt-shelf-notice-title">{IDENTITY_TITLE}</p>
      <p className="cdt-shelf-notice-body">{IDENTITY_BODY_1}</p>
      <p className="cdt-shelf-notice-body">{IDENTITY_BODY_2}</p>
      <ul className="cdt-fr-identity-rows">
        {props.rows.map((row) => {
          const on = !unticked.has(row.email);
          return (
            <li key={row.email} className="cdt-fr-identity-row" data-on={on}>
              <input
                type="checkbox"
                className="cdt-fr-identity-tick"
                checked={on}
                aria-label={row.email}
                onChange={() => {
                  toggle(row.email);
                }}
              />
              <span className="cdt-fr-identity-text">
                {/* An address is a machine string, and a wrapped one reads as two addresses. */}
                <span className="cdt-fr-identity-email" data-on={on}>
                  {row.email}
                </span>
                <span className="cdt-fr-identity-source">{identitySourceLine(row)}</span>
              </span>
            </li>
          );
        })}
      </ul>
      {effect === null ? null : <p className="cdt-fr-identity-effect">{effect}</p>}
      <div className="cdt-shelf-notice-actions">
        <button
          type="button"
          className="cdt-shelf-notice-primary"
          onClick={() => {
            props.onConfirm(ticked);
          }}
        >
          {IDENTITY_CONFIRM_LABEL}
        </button>
        <button
          type="button"
          className="cdt-shelf-notice-secondary"
          onClick={() => {
            props.onLeaveAsIs();
          }}
        >
          {IDENTITY_LEAVE_LABEL}
        </button>
      </div>
      <p className="cdt-fr-identity-footnote">{IDENTITY_FOOTNOTE}</p>
    </div>
  );
}

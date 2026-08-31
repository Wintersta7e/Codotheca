import type { CSSProperties, ReactElement, ReactNode } from 'react';

/**
 * §8.0b's chip box, as one component.
 *
 * It was written inline inside `AttentionRow` for the four built-ins, which have no accessible
 * name of their own, always have a number, and always have a plain-text sub-line. §8.8's saved
 * chips need none of those to hold: a broken collection has **no** number, a degraded one strikes
 * its dropped terms *inside* the sub-line, and every collection chip is named by what it counted.
 * §8.8 refuses a second box in the plainest terms — *"one box written in three places is one box
 * that will be changed in one"* — so the box moved here and both surfaces render it.
 *
 * The chrome itself is `shelf.css`'s and is not restated in this file or in `collections.css`.
 */

/** §8.0b's `ALL` row and §8.8's collection chips both resolve to the neutral. */
export const DEFAULT_CHIP_ACCENT = 'var(--text-2)';

export interface AttentionChipProps {
  /**
   * `null` renders **no number node at all** — not `0`, not an em dash, not a dimmed digit.
   * §8.8's broken chip counted nothing, and a blank where a figure belongs is the only reading
   * that is not a claim.
   */
  readonly count: number | null;
  readonly label: string;
  /**
   * A `ReactNode` rather than a string: §8.3a's soft-error rendering strikes the dropped terms
   * inside the sub-line, and flattening that to text loses the strike.
   */
  readonly subLine: ReactNode;
  /** §8.0b's `inset 3px 0 0 0 <accent>`, reaching the stylesheet as `--cdt-chip-accent`. */
  readonly accent?: string;
  readonly active: boolean;
  /** §8.8: a broken chip cannot be activated, because there is no query left to activate. */
  readonly disabled?: boolean;
  /**
   * §11.7: spec content, never a developer's invention. Omitted for the four built-ins, whose
   * visible label *is* their name; §8.8 states one verbatim for each collection chip state.
   */
  readonly accessibleName?: string;
  readonly onActivate: () => void;
}

export function AttentionChip(props: AttentionChipProps): ReactElement {
  const accent = props.accent ?? DEFAULT_CHIP_ACCENT;
  return (
    <button
      type="button"
      className="cdt-attention-chip"
      aria-pressed={props.active}
      aria-label={props.accessibleName}
      disabled={props.disabled ?? false}
      style={{ '--cdt-chip-accent': accent } as CSSProperties}
      onClick={props.onActivate}
    >
      {props.count === null ? null : (
        <span className="cdt-attention-count" data-part="count">
          {props.count}
        </span>
      )}
      <span>
        <span className="cdt-attention-label">{props.label}</span>
        <span className="cdt-attention-sub">{props.subLine}</span>
      </span>
    </button>
  );
}

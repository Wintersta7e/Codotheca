import { type ReactElement, useLayoutEffect, useRef, useState } from 'react';
import { eraChevronName } from '../a11y/names.js';
import { flagLineText, summaryText, summaryTextTruncated } from './eras.js';
import type { ShelfSection } from './page.js';

export type OverflowProbe = (el: HTMLElement) => boolean;

/** Injected so the truncation rule is testable without a layout engine. */
export const measuresOverflow: OverflowProbe = (el) => el.scrollWidth > el.clientWidth;

export interface EraHeaderProps {
  readonly section: ShelfSection;
  readonly collapsed: boolean;
  /**
   * §8.5.1: the header of the section you land back in lights along its length, so the shelf says
   * which one you returned to rather than leaving you to find it. Exactly one section flares.
   */
  readonly flaring?: boolean;
  readonly onToggle: () => void;
  readonly probe?: OverflowProbe;
}

export function EraHeader(props: EraHeaderProps): ReactElement {
  const { section, collapsed, flaring = false, onToggle, probe = measuresOverflow } = props;
  const summaryRef = useRef<HTMLSpanElement | null>(null);

  const full = summaryText(section.agg);
  const flags = flagLineText(section.agg);

  /**
   * The *string* the decision was made about, not a boolean. Held this way so a changed aggregate
   * re-measures on its own — and so the measurement only ever runs against the full form. Once
   * truncated the short string fits by construction, so re-probing it would restore the long one
   * and the two would alternate forever.
   */
  const [truncatedFor, setTruncatedFor] = useState<string | null>(null);
  const truncated = truncatedFor === full;

  useLayoutEffect(() => {
    const el = summaryRef.current;
    if (!el || truncated) return;
    if (probe(el)) setTruncatedFor(full);
  }, [full, probe, truncated]);

  return (
    <div className="cdt-era-header">
      <button
        type="button"
        className="cdt-era-chevron"
        aria-expanded={!collapsed}
        aria-label={eraChevronName(`${section.label} · ${full}`)}
        onClick={onToggle}
      >
        <span className="cdt-era-label">{section.label}</span>
        {/* The only element that ellipsises. Ellipsis cuts from the end, where §8.1's coverage
            parenthetical sits, and `41 GB tracked (of 198 ind…` reads as a complete figure — so
            the byte aggregate and its parenthetical go as one unit and the count never does. */}
        <span ref={summaryRef} className="cdt-era-summary">
          {truncated ? summaryTextTruncated(section.agg) : full}
        </span>
        {/* `flex:none`, and never truncated: an absent flag line may only mean *observed, and
            nothing to report*. */}
        {flags === null ? null : <span className="cdt-era-flags">{flags}</span>}
      </button>
      {/* Outside the button: it is light, not a control, and it must never be in the accessible
          name of one. Rendered only while it plays, so nothing inert is left behind it. */}
      {flaring ? <span className="cdt-era-flare" data-gesture="flare" aria-hidden="true" /> : null}
    </div>
  );
}

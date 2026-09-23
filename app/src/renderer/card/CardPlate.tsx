import type { ReactElement, ReactNode } from 'react';
import { ARCHIVED_GLASS } from './completion';
import { type CardSurface, bandsFor } from './geometry';

/**
 * Everything inside the 1px tier frame. `card.css` gives `.cdt-plate` its clip, its bezel and
 * §7.3a's two background images through `--cdt-plate` and `--cdt-greebling`; this component
 * decides only the stack.
 *
 * §7.1a's bitmap is the caller's node and sits **first**, directly above the CSS plate and below
 * every band, because the raster carries no text — type is DOM. When there is no bitmap the CSS
 * plate is not a placeholder: it is the T0 state and §7.5's finished fallback.
 *
 * The only layers that legitimately span the plate are the ones carrying no content (§7.7). The
 * bezel, the hazard tape and the label scrim are the others, and they are the shell's and the
 * bands', not this component's.
 */
export interface CardPlateProps {
  readonly surface: CardSurface;
  readonly isArchived: boolean;
  /** §7.1a's decoded bitmap, or `null` for §7.3a's CSS plate alone. */
  readonly art: ReactNode;
  /** §7.8's watermark. No phase-1 producer emits one; the rule is built and nothing mounts it. */
  readonly sigil?: ReactNode;
  /**
   * [p3] §33.4's five material layers. Mounted **after `.cdt-scanline` and before `.cdt-glass`**
   * — above the bitmap and above every plate-spanning light effect, below all five bands, so no
   * chip, stripe, glyph or string is ever dimmed by decay. `.cdt-glass` mounts only when
   * archived, so the position is a slot here rather than an insertion relative to an element
   * that may not exist.
   *
   * Only the opened hero hands one over. A surface that passes none renders none, which is the
   * correct default: decay is never on the grid, in Peek, in the list, in the palette or in
   * triage — the same scope roasting has.
   */
  readonly decay?: ReactNode;
  /**
   * [p3] §34.1's restoration surge, mounted **immediately after the bitmap** so it is a sibling of
   * `.cdt-art` inside the plate, and therefore inside the card subtree `motion.css`'s `off`
   * blanket reaches. Only the opened hero hands one over: a restoration on the grid is the
   * existing `condition_changed` repaint and nothing more — *Signal on the grid, Workshop on the
   * page*.
   */
  readonly surge?: ReactNode;
  /** The five bands. */
  readonly children: ReactNode;
}

export function CardPlate(props: CardPlateProps): ReactElement {
  const bands = bandsFor(props.surface);
  return (
    <div className="cdt-plate">
      {props.art}
      {props.surge}
      <span
        className="cdt-sheen"
        aria-hidden="true"
        style={{
          backgroundImage: `linear-gradient(157deg, rgb(255 255 255 / ${bands.sheenAlpha}), transparent 44%)`,
        }}
      />
      <span className="cdt-specular" aria-hidden="true" />
      <span className="cdt-scanline" aria-hidden="true" />
      {props.decay}
      {props.isArchived ? (
        // §7.7a's sealed marker. The value is `completion.ts`'s, so the fallback rule in
        // `card.css` cannot become a second, differently-spelled gradient.
        <span
          className="cdt-glass"
          aria-hidden="true"
          style={{ backgroundImage: ARCHIVED_GLASS }}
        />
      ) : null}
      {props.sigil === undefined ? null : (
        <span className="cdt-sigil" aria-hidden="true">
          {props.sigil}
        </span>
      )}
      {props.children}
    </div>
  );
}

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
  /** The five bands. */
  readonly children: ReactNode;
}

export function CardPlate(props: CardPlateProps): ReactElement {
  const bands = bandsFor(props.surface);
  return (
    <div className="cdt-plate">
      {props.art}
      <span
        className="cdt-sheen"
        aria-hidden="true"
        style={{
          backgroundImage: `linear-gradient(157deg, rgb(255 255 255 / ${bands.sheenAlpha}), transparent 44%)`,
        }}
      />
      <span className="cdt-specular" aria-hidden="true" />
      <span className="cdt-scanline" aria-hidden="true" />
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

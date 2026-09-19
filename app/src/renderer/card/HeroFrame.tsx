import type { ReactElement, ReactNode } from 'react';
import type { ProjectRow } from '../../generated/protocol';
import { appearanceFor, fadeFor, languageCode, seedOf } from '../art/appearance';
import { renditionFor, useCardBitmap } from '../art/useCardBitmap';
import { Card, type CardHalo } from './Card';
import type { StatusChip } from './chips';
import { frameToken, rungFor, uncomputedRank } from './completion';
import type { PinControlProps } from './PinControl';

/**
 * §7.7's **second** band table, mounted. The hero is not the tile at hero scale: building that
 * sentence literally moves the jewel stripe out of the band it closes, so this component passes
 * `surface="hero"` and reads nothing from the tile's table.
 *
 * Band 5 is `children`. §8.5 owns its content, and the project page is the only caller.
 *
 * **Band 1 owns the pin, on both tile and hero.** §7.8a gives the hero its own box, ground, glyph,
 * rotation, ink and hit-target size, every one of them different from the tile's, and none of that
 * would be specified for a mark that never renders. Its closing sentence rules out a **second
 * copy** — a pin in the page's chrome on top of this one — not this one.
 *
 * The address arrives from the caller because `art.url {rendition:'hero'}` *is* the demand that
 * renders it — the core cannot observe an open page — and `''` means keep the plate.
 */
export type HeroRow = Pick<
  ProjectRow,
  | 'seedBasename'
  | 'rerollOffset'
  | 'primaryLanguage'
  | 'isReference'
  | 'isArchived'
  | 'conditionSignal'
  | 'completionLit'
  // [p3] §31.1c: the frame reads `pct = lit / evaluable`, so the denominator has to reach the
  // hero as well as the tile. `ProjectRow` already carried both.
  | 'completionApplicable'
  | 'artSceneHash'
  | 'artState'
  // §23.5: the hero asks for `hero-blueprint` and takes the blueprint frame when the project has
  // no working copy. §23.1's one predicate, so the row has to carry it.
  | 'primaryLocation'
>;

export interface HeroFrameProps {
  readonly row: HeroRow;
  /** The `art.url` answer. `''` is "no address": §7.5's nameplate stands. */
  readonly heroSrc: string;
  readonly halo: CardHalo;
  /**
   * §7.7's band 4 is one table for both surfaces — "status chips only" — so this is the strip
   * `statusChips` produces, handed in by the caller because the `NEW` boundary is a library-wide
   * fact no single project's payload carries.
   */
  readonly chips: readonly StatusChip[];
  /** §7.8a's band-1 mark, at the hero's own values. `null` draws none. */
  readonly pin: PinControlProps | null;
  readonly children: ReactNode;
}

/**
 * The hero is one size and §7.7's second table gives it fixed figures; `densityStep` is read
 * only for the tile's three name and glyph sizes, which `card.css` overrides for
 * `[data-surface='hero']` anyway.
 */
const HERO_DENSITY = 186;

export function HeroFrame(props: HeroFrameProps): ReactElement {
  const { row } = props;
  const appearance = appearanceFor(seedOf(row), fadeFor(row), row.primaryLanguage);
  const hasWorkingCopy = row.primaryLocation !== null;
  // [p3] §31.1c: `pct = lit / evaluable` and nothing else. `null` is one of the three cases
  // decided above the ladder, and `uncomputedRank` below draws that one.
  const rung = rungFor({
    completionLit: row.completionLit,
    completionApplicable: row.completionApplicable,
    isReference: row.isReference,
    hasWorkingCopy,
    isArchived: row.isArchived,
  });
  const bitmap = useCardBitmap({
    sceneHash: row.artSceneHash,
    rendition: renditionFor('hero', hasWorkingCopy),
    artState: row.artState,
    src: props.heroSrc,
  });

  return (
    <Card
      surface="hero"
      appearance={appearance}
      // [p3] §31.1c. `rungFor` answers `null` for the three cases decided ABOVE the ladder —
      // Reference, no working copy, and an uncomputed measurement — which is exactly where
      // §7.7a's own frame applies, so the two never both answer.
      frameToken={rung?.frameToken ?? frameToken({ isReference: row.isReference, hasWorkingCopy })}
      notched={rung?.notched ?? false}
      density={HERO_DENSITY}
      isArchived={row.isArchived}
      isReference={row.isReference}
      halo={props.halo}
      hovered={false}
      focused={false}
      selected={false}
      art={
        bitmap.src === null ? null : (
          <img
            className="cdt-art"
            alt=""
            src={bitmap.src}
            decoding="async"
            style={{
              position: 'absolute',
              inset: 0,
              width: '100%',
              height: '100%',
              display: 'block',
            }}
          />
        )
      }
      bands={{
        languageCode: row.primaryLanguage === null ? null : languageCode(row.primaryLanguage),
        designation: appearance.designation,
        hazard: row.conditionSignal === 'abandoned' && !row.isReference,
        conditionSignal: row.conditionSignal,
        rank: uncomputedRank('hero', {
          completionLit: row.completionLit,
          isReference: row.isReference,
          hasWorkingCopy,
          density: HERO_DENSITY,
        }),
        // §7.7 gives the hero column a geometry and §8.5 owns what goes in it.
        chips: props.chips,
        pin: props.pin,
      }}
    >
      {props.children}
    </Card>
  );
}

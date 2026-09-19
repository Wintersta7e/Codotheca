/**
 * §8.5.1's 268px hero. It composes §7.7's **second** band table rather than restating it: the
 * hero is not the tile at hero scale, and a band edge written in two places is one that drifts
 * on the first correction.
 *
 * What this component owns is what §8.5 owns — band 5's content, the band-4 strip and the
 * address. Everything else (frame, gap, hairline, rank, vent, jewel stripe, scrim) is the shared
 * card shell, which is also what keeps the hero inside §11.6's tier clamp.
 *
 * **[p3] All five material layers are IN — §33.4 supersedes the sentence below.** They mount
 * through `HeroFrame`'s `decay` slot, **on this surface and nowhere else**, lit by the open debt
 * list rather than by a clock: every layer has a trigger now, so the argument for shipping none
 * of them expires with it. The ambient key-light drift is still out of phase 1 entire and §7.8's
 * hover effects are still grid effects, so this tile emits **zero animation frames** once the
 * page's entry has played — the layers transition on opacity and animate nothing.
 *
 * ~~All five material layers are out — dust, cobwebs, rust, cracks, overgrowth. Only cobwebs has
 * a phase-1 trigger, and shipping one layer of five would read as a bug in the decay model
 * rather than as its absence.~~
 *
 * Band 5 keeps the identity line and drops the `<lit>/<applicable> EVALUABLE` score outright —
 * not an em dash either, because band 3 already reads `RANK NOT COMPUTED` and one surface states
 * an absence once.
 */
import type { ReactElement } from 'react';

import type { ProjectRow, SceneHash } from '../../../generated/protocol';
import { statusChips } from '../../card/chips';
import { HeroFrame } from '../../card/HeroFrame';
import { glowShadow, glowStrength } from '../../derive/condition';
import { useProjectPageDeps } from '../deps';
import { renditionFor } from '../../art/useCardBitmap';
import { useHeroArt } from './useHeroArt';

export interface HeroTileProps {
  row: ProjectRow;
  /** The hash the page is holding: the row's, or a later one `art_ready` announced. */
  heroHash: SceneHash | null;
  /**
   * §10.5a's boundary. `null` is "first run has not finished", under which nothing is new — so
   * a page with no source for the stamp draws no `NEW`, which is the documented reading and not
   * a guess.
   */
  firstRunCompletedAt: number | null;
  /**
   * The pinned bit **as the page is drawing it**, which is not always `row.isPinned`: the
   * renderer flips it in its own projection the moment the control is pressed, because the shelf
   * filters `is:pinned` client-side and a mark that waits for a round trip leaves the query
   * disagreeing with the mark in the meantime.
   */
  isPinned: boolean;
  onTogglePin: () => void;
}

/**
 * §7.7's band-5 identity line: `<first-commit year> · <primary language>`, with `owner` prefixed
 * only when it is not the user — which the projection expresses by leaving `owner` NULL when it
 * is. Whichever half is NULL is omitted; both NULL renders nothing at all.
 */
export function heroIdentityLine(row: ProjectRow): string | null {
  const parts = [
    row.owner === null ? null : row.owner.toUpperCase(),
    row.birthYear === null ? null : String(row.birthYear),
    row.primaryLanguage === null ? null : row.primaryLanguage.toUpperCase(),
  ].filter((part): part is string => part !== null && part !== '');
  return parts.length === 0 ? null : parts.join(' · ');
}

export function HeroTile({
  row,
  heroHash,
  firstRunCompletedAt,
  isPinned,
  onTogglePin,
}: HeroTileProps): ReactElement {
  const deps = useProjectPageDeps();
  // §23.5: the hero asks the core for the pass it needs, and that request **is** the demand that
  // renders it. `renditionFor` names the pass; §23.1's one predicate decides which.
  const heroSrc = useHeroArt(
    heroHash,
    row.artState,
    renditionFor('hero', row.primaryLocation !== null),
  );
  const identity = heroIdentityLine(row);

  // §5.4a's steady glow. There is no session on this surface to raise it and no flicker to dip
  // it: the scheduled flicker is a grid effect, so the halo stands at 1.
  const glow = glowShadow(
    glowStrength({
      signal: row.conditionSignal,
      isReference: row.isReference,
      isArchived: row.isArchived,
      hasOpenSession: false,
    }),
  );

  return (
    <HeroFrame
      row={{ ...row, artSceneHash: heroHash }}
      heroSrc={heroSrc}
      halo={{ shadow: glow, opacity: 1 }}
      chips={statusChips(row, deps.now(), firstRunCompletedAt)}
      pin={{
        projectName: row.name,
        isPinned,
        surface: 'hero',
        // The grid reveals the empty mark on hover or focus because it draws 180 of them. This
        // surface draws one, and nothing here hovers: §7.8's hover choreography is a grid effect
        // and §8.5.1 holds this page to zero animation frames once its entry has played. A mark
        // that only appeared on a hover the page does not track would be unreachable.
        visible: true,
        // No grid here to hold a roving tabindex, and this page binds no `P`, so a real tab stop
        // is the only way the control can be reached without a pointer.
        tabIndex: 0,
        onToggle: onTogglePin,
      }}
    >
      {identity === null ? null : (
        <div className="cp-hero-identity" data-testid="cp-hero-identity">
          {identity}
        </div>
      )}
    </HeroFrame>
  );
}

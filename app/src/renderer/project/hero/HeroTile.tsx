/**
 * §8.5.1's 268px hero. It composes §7.7's **second** band table rather than restating it: the
 * hero is not the tile at hero scale, and a band edge written in two places is one that drifts
 * on the first correction.
 *
 * What this component owns is what §8.5 owns — band 5's content, the band-4 strip and the
 * address. Everything else (frame, gap, hairline, rank, vent, jewel stripe, scrim) is the shared
 * card shell, which is also what keeps the hero inside §11.6's tier clamp.
 *
 * **All five material layers are out** — dust, cobwebs, rust, cracks, overgrowth. Only cobwebs
 * has a phase-1 trigger, and shipping one layer of five would read as a bug in the decay model
 * rather than as its absence. The ambient key-light drift is out of phase 1 entire and §7.8's
 * hover effects are grid effects, so this tile emits **zero animation frames** once the page's
 * entry has played.
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

export function HeroTile({ row, heroHash, firstRunCompletedAt }: HeroTileProps): ReactElement {
  const deps = useProjectPageDeps();
  const heroSrc = useHeroArt(heroHash, row.artState);
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
    >
      {identity === null ? null : (
        <div className="cp-hero-identity" data-testid="cp-hero-identity">
          {identity}
        </div>
      )}
    </HeroFrame>
  );
}

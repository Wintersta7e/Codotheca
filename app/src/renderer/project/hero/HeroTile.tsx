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
 * **[p3] §34.6: that sentence bans a schedule, not a response.** The restoration surge mounts
 * through `HeroFrame`'s `surge` slot for a change the user is present for, plays one bounded
 * envelope and ends — the specular sweep's carve-out, not the key-light drift's revival.
 *
 * ~~All five material layers are out — dust, cobwebs, rust, cracks, overgrowth. Only cobwebs has
 * a phase-1 trigger, and shipping one layer of five would read as a bug in the decay model
 * rather than as its absence.~~
 *
 * **[p3] §31.7: band 5 renders the score when the figure exists**, and keeps the sentence above
 * when it does not. Phase 1 dropped it outright because nothing computed one — not an em dash
 * either, because band 3 already reads `RANK NOT COMPUTED` and one surface states an absence
 * once. That reason expires with §31; the absence rule does not.
 */
import { useRef, type ReactElement } from 'react';

import type { DebtItem, ProjectId, ProjectRow, SceneHash } from '../../../generated/protocol';
import { DecayStack } from '../../decay/DecayStack';
import { useWeathering } from '../../decay/useWeathering';
import { originFor, type Origin } from '../../restoration/origin';
import type { Selection } from '../../restoration/select';
import { Surge, type SurgeRequest } from '../../restoration/Surge';
import { statusChips } from '../../card/chips';
import { scoreText } from '../../card/completion';
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
  /**
   * [p3] §31.7's `· <n> UNKNOWN` clause. It renders only on surfaces served by `projects.get`,
   * so the page hands the count down rather than the hero inventing one. `null` is *the detail
   * has not arrived*, which renders the score without the clause rather than with a zero.
   */
  unknownChecks?: number | null;
  onTogglePin: () => void;
  /**
   * [p3] §28's flat debt list, straight off `ProjectDetail`. §33.2 counts the open items per
   * layer; the anchors arrive separately, from the core, keyed by scene.
   */
  debt?: readonly DebtItem[];
  /**
   * [p3] §34's restoration, as the page decided it at the event's arrival. `null` or absent draws
   * nothing. The hero is the only surface that takes one.
   */
  surge?: SurgeRequest | null;
}

/**
 * §7.7's band-5 identity line: `<first-commit year> · <primary language>`, with `owner` prefixed
 * only when it is not the user — which the projection expresses by leaving `owner` NULL when it
 * is. Whichever half is NULL is omitted; both NULL renders nothing at all.
 */
/**
 * [p3] §33.4's stack, mounted against the hash **actually on screen**.
 *
 * A component rather than an expression because `useWeathering` is a hook: `HeroFrame` hands the
 * decoded hash to a render prop, and a hook cannot be called inside a callback.
 */
function HeroDecay(props: {
  projectId: ProjectId;
  decodedSceneHash: SceneHash | null;
  debt: readonly DebtItem[];
}): ReactElement | null {
  const weathering = useWeathering(props.projectId, props.decodedSceneHash);
  return <DecayStack weathering={weathering} debt={props.debt} />;
}

/**
 * [p3] §34.5's surge, against the hash **actually on screen**, for `HeroDecay`'s reason.
 *
 * **Mounted for as long as the page is, not only while a surge plays**, so the anchor set is
 * already here when a restoration arrives: a surge that fetched its anchors on arrival would
 * start as the wipe and turn into the front part-way through.
 *
 * **The origin is fixed for the life of one selection.** An anchor reply that lands mid-surge must
 * not change the rendering under the animation — the selection's own origin, or its absence, is
 * the whole of what that surge says.
 */
function HeroSurge(props: {
  projectId: ProjectId;
  decodedSceneHash: SceneHash | null;
  request: SurgeRequest;
}): ReactElement | null {
  const weathering = useWeathering(props.projectId, props.decodedSceneHash);
  const fixed = useRef<{ selection: Selection; origin: Origin | null } | null>(null);
  const { selection, end } = props.request;
  if (fixed.current?.selection !== selection) {
    fixed.current = {
      selection,
      origin: selection.kind === 'layer' ? originFor(weathering, selection.layer) : null,
    };
  }
  return <Surge selection={selection} origin={fixed.current.origin} onEnd={end} />;
}

export function heroIdentityLine(row: ProjectRow): string | null {
  const parts = [
    row.owner === null ? null : row.owner.toUpperCase(),
    row.birthYear === null ? null : String(row.birthYear),
    row.primaryLanguage === null ? null : row.primaryLanguage.toUpperCase(),
  ].filter((part): part is string => part !== null && part !== '');
  return parts.length === 0 ? null : parts.join(' · ');
}

/**
 * [p3] §31.7's band-5 readout: `<lit>/<evaluable> EVALUABLE · <n> UNKNOWN`.
 *
 * **The second clause drops only at `n === 0`.** Dropping it unconditionally hides a real
 * quantity; rendering it unconditionally prints `· 0 UNKNOWN` on a fully observed project, which
 * is furniture. `null` is the uncomputed case, where band 3 already states the absence.
 *
 * **Never a percentage and never a bare numerator** — the denominator is the whole safeguard.
 */
export function heroScoreLine(
  lit: number | null,
  evaluable: number | null,
  unknown: number,
): string | null {
  const score = scoreText(lit, evaluable);
  if (score === null) return null;
  return unknown === 0 ? `${score} EVALUABLE` : `${score} EVALUABLE · ${String(unknown)} UNKNOWN`;
}

export function HeroTile({
  row,
  heroHash,
  firstRunCompletedAt,
  isPinned,
  unknownChecks,
  onTogglePin,
  debt = [],
  surge = null,
}: HeroTileProps): ReactElement {
  const deps = useProjectPageDeps();
  // §23.5: the hero asks the core for the pass it needs, and that request **is** the demand that
  // renders it. `renditionFor` names the pass; §23.1's one predicate decides which.
  const rendition = renditionFor('hero', row.primaryLocation !== null);
  const heroSrc = useHeroArt(heroHash, row.artState, rendition);
  // [p3] §33.4: **never on a blueprint pass.** `hero-blueprint` (§23.5) is line art with no
  // machined parts to weather, and a zero-location project has no working copy for §29's or
  // §32's readers to observe. The pass `renditionFor` already chose is read here, never
  // re-derived from `primaryLocation` a second time.
  const isBlueprint = rendition === 'hero-blueprint';
  const identity = heroIdentityLine(row);
  // §31.7: the figure comes from the row, which is what decides *is there a figure at all*. The
  // `UNKNOWN` clause needs a count only `projects.get` carries, so it is the caller's.
  const score = heroScoreLine(row.completionLit, row.completionApplicable, unknownChecks ?? 0);

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
      decay={(decodedSceneHash) =>
        isBlueprint ? null : (
          <HeroDecay projectId={row.id} decodedSceneHash={decodedSceneHash} debt={debt} />
        )
      }
      // [p3] §34: never on a blueprint pass either — line art with no machined parts has no
      // working copy whose debt could have been restored.
      surge={(decodedSceneHash) =>
        isBlueprint || surge === null ? null : (
          <HeroSurge projectId={row.id} decodedSceneHash={decodedSceneHash} request={surge} />
        )
      }
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
      {score === null ? null : (
        <div className="cp-hero-score" data-testid="cp-hero-score">
          {score}
        </div>
      )}
    </HeroFrame>
  );
}

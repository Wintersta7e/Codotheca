import { LADDER_RUNGS, type TokenName } from '../theme/tokens';
import { densityStep } from './geometry';

/**
 * §7.7a. Nothing in phase 1 writes `completion_lit`, so the ladder has no input at all — on
 * every card. This is not an edge case to degrade into; it is the only case the phase ships,
 * and it is drawn as a finished state.
 *
 * The generating rule: *no surface may render a member of the completion ladder while
 * `completion_lit IS NULL`*. Plain `#333c45` is a rung, not a neutral — painting it asserts
 * "measured, and under half". Silver sits below `pct >= 1`, so an archived project drawn silver
 * asserts "archived and not 10 of 10" from no measurement at all.
 */
export const EM_DASH = '—';

export type CompletionSurface = 'gridCard' | 'hero' | 'listRow' | 'paletteRow';

export interface FrameGap {
  readonly left: string;
  readonly top: string;
  readonly width: string;
  readonly height: string;
  readonly fillToken: 'surface-1' | 'surface-0';
}

export type FrameToken = 'unknown' | 'tier-ref' | 'tier-blue';

export interface UncomputedRank {
  readonly frameToken: FrameToken;
  readonly gap: FrameGap;
  readonly glyph: string;
  readonly glyphPx: number;
  readonly glyphInkToken: 'unknown-ink';
  readonly label: 'NOT COMPUTED' | 'RANK NOT COMPUTED';
  readonly labelPx: number;
  readonly labelInkToken: 'text-3';
  readonly accessibleName: 'Completion not computed';
}

export interface CompletionInput {
  readonly completionLit: number | null;
  readonly isReference: boolean;
  /**
   * §23.5: `primaryLocation !== null`, the one predicate §23.1 names. The blueprint frame is
   * decided from *is there a working copy at all*, which is a fact about the row and not a
   * measurement of it.
   */
  readonly hasWorkingCopy: boolean;
  /** The `--tile` value in px. Ignored on the hero, which is one size. */
  readonly density: number;
}

/**
 * `is_reference → --tier-ref` is decided *above* the ladder, from `authored_by_user` (§5.5),
 * and §5.5 writes `is_reference = 1` only from a computed `authored_by_user`. A row whose
 * authorship has not been computed therefore arrives here as `isReference === false` and takes
 * the unknown frame, which is what §7.7a asks for.
 *
 * **§23.5 makes `--tier-blue` reachable, and it is not a rung.** §7.7's generating rule — *no
 * surface may render a member of the completion ladder while `completion_lit IS NULL`* — is not
 * breached, because this is decided **above** the ladder from *is there a working copy at all*,
 * exactly as `--tier-ref` is decided above it from authorship. `paintsLadderRung('#2f4a5c')` is
 * false, and it stays false.
 *
 * **`isReference` is tested first**, and §23 does not rule the pair. The reading taken: a
 * reference row renders in §8.1's own Reference chrome below the grid rather than in the
 * not-cloned tail, so its frame is that chrome's. In practice the two are disjoint for a project
 * that was never cloned — `is_reference` is written only from a computed `authored_by_user`,
 * which needs commits — so the ordering is observable only for a project that was cloned, marked
 * reference and later uninstalled, which is p2-24b's state and not §23's.
 */
export function frameToken(
  input: Pick<CompletionInput, 'isReference' | 'hasWorkingCopy'>,
): FrameToken {
  if (input.isReference) return 'tier-ref';
  if (!input.hasWorkingCopy) return 'tier-blue';
  return 'unknown';
}

/** §7.7a: the notch qualifies a measurement, the gap says there is none. Never both. */
export const GOLD_NOTCH = {
  right: '11px',
  top: '-1px',
  width: '9px',
  height: '3px',
} as const;

/** §7.7a: `is_archived` keeps only its sealed markers, which are independent of completion. */
export const ARCHIVED_GLASS =
  'linear-gradient(122deg, rgb(255 255 255 / .16) 0%, transparent 26%, transparent 64%, rgb(255 255 255 / .07) 100%)';

const HERO_GLYPH_PX = 64;

const RUNGS = new Set(LADDER_RUNGS.map((rung) => rung.toLowerCase()));

/** A guard for the invariant, usable from any surface that computes a colour. */
export function paintsLadderRung(colour: string): boolean {
  return RUNGS.has(colour.trim().toLowerCase());
}

/**
 * Returns the treatment for a slot that is **drawn anyway**, and `null` for a slot that exists
 * only to carry the value. `null` means *render no node*, not *render an em dash*: a dash placed
 * in a column that could simply have been empty is a claim with no measurement behind it, and it
 * passes a digits-only check while doing it.
 *
 * `gap.top` is `-1px`, so the node the caller builds from it mounts on the **unclipped**
 * `.cdt-card-frame` and never inside `.cdt-card`, whose `clip-path` would delete it exactly as
 * it deletes an `outline`. `card.css` says the same beside `.cdt-frame-gap`.
 */
export function uncomputedRank(
  surface: CompletionSurface,
  input: CompletionInput,
): UncomputedRank | null {
  if (input.completionLit !== null) return null;
  if (surface !== 'gridCard' && surface !== 'hero') return null;
  const hero = surface === 'hero';
  return {
    frameToken: frameToken(input),
    gap: {
      left: '39%',
      top: '-1px',
      width: '22%',
      height: '3px',
      // §8.7 snaps `#0b0e11` onto `--surface-0` and names the hero band fill as one of the four
      // sites that travel with it, so the fill is a token name here and never a hex.
      fillToken: hero ? 'surface-0' : 'surface-1',
    },
    glyph: EM_DASH,
    glyphPx: hero ? HERO_GLYPH_PX : densityStep(input.density).glyphPx,
    glyphInkToken: 'unknown-ink',
    label: hero ? 'RANK NOT COMPUTED' : 'NOT COMPUTED',
    labelPx: hero ? 7.5 : 7,
    labelInkToken: 'text-3',
    accessibleName: 'Completion not computed',
  };
}

/**
 * [p3] §31.1c's six rungs. `goldArchived` is a rung of its own and not gold with a modifier —
 * the two differ in frame **and** ink.
 */
export type Rung = 'gold' | 'goldArchived' | 'silver' | 'brass' | 'steel' | 'plain';

export interface RungPaint {
  readonly rung: Rung;
  readonly frameToken: TokenName;
  readonly inkToken: TokenName;
  /**
   * §31.1c: the notch fires on `evaluable < 10`, **whether the denominator shrank because a
   * check is `na` or because a check is `unknown`**. Plain gold keeps meaning *ten of ten*;
   * notched gold means *100% of what could be scored*, which an offline and unauthenticated user
   * can still reach.
   *
   * It is only ever true on a gold rung, and it is **never drawn beside §7.7a's gap** — the gap
   * says there is no measurement and the notch qualifies one, so they mean opposite things and a
   * card showing both would be saying both.
   */
  readonly notched: boolean;
}

const RUNG_PAINT: Readonly<Record<Rung, { frameToken: TokenName; inkToken: TokenName }>> = {
  gold: { frameToken: 'tier-gold', inkToken: 'tier-gold-ink' },
  goldArchived: { frameToken: 'tier-gold-archived', inkToken: 'tier-gold-archived-ink' },
  silver: { frameToken: 'tier-silver', inkToken: 'tier-silver-ink' },
  brass: { frameToken: 'tier-brass', inkToken: 'tier-brass-ink' },
  steel: { frameToken: 'tier-steel', inkToken: 'tier-steel-ink' },
  plain: { frameToken: 'tier-plain', inkToken: 'tier-plain-ink' },
};

export interface RungInput {
  readonly completionLit: number | null;
  readonly completionApplicable: number | null;
  readonly isReference: boolean;
  readonly hasWorkingCopy: boolean;
  readonly isArchived: boolean;
}

/**
 * [p3] §31.1c's table, **tested in this order**, because each row makes the next meaningless if
 * it is answered the other way.
 *
 * | Condition | Frame |
 * |---|---|
 * | `is_reference` | `--tier-ref`, **above** the ladder |
 * | no working copy | `--tier-blue`, **above** the ladder |
 * | `completion_lit IS NULL` | §7.7a's unknown frame, in full |
 * | `pct >= 1` | gold, and **archived gold when `is_archived`** |
 * | `is_archived` | silver |
 * | `pct >= 0.8` | brass |
 * | `pct >= 0.5` | steel |
 * | otherwise | plain |
 *
 * **The frame reads `pct = lit / evaluable` and nothing else** — material means exactly one
 * thing. `null` on either column is *uncomputed*, which is §7.7a's treatment in full and is what
 * `uncomputedRank` returns; this function answers `null` there so the caller keeps one path to
 * that treatment instead of two.
 *
 * **Silent demotion.** Connecting an account may lower the tier — notched gold `7/7` to brass
 * `8/9` — and the demotion carries no animation, no `health_delta` row, no XP reversal and no
 * notification. *A measurement is not an earned thing*: `concept.md`'s *nothing earned ever
 * removed* governs the XP ledger and badges, which are latched. The frame is a live measurement,
 * and a measurement that refuses to change when new information arrives is the thing this
 * product bans everywhere else.
 */
export function rungFor(input: RungInput): RungPaint | null {
  const { completionLit: lit, completionApplicable: evaluable } = input;
  if (input.isReference || !input.hasWorkingCopy) return null;
  if (lit === null || evaluable === null || evaluable <= 0) return null;

  const pct = lit / evaluable;
  const notched = evaluable < 10;
  let rung: Rung;
  if (pct >= 1) {
    rung = input.isArchived ? 'goldArchived' : 'gold';
  } else if (input.isArchived) {
    rung = 'silver';
  } else if (pct >= 0.8) {
    rung = 'brass';
  } else if (pct >= 0.5) {
    rung = 'steel';
  } else {
    rung = 'plain';
  }
  return {
    rung,
    ...RUNG_PAINT[rung],
    // The notch qualifies a 100% measurement. On any lower rung the fraction already carries
    // its own denominator, so there is nothing for it to qualify.
    notched: notched && (rung === 'gold' || rung === 'goldArchived'),
  };
}

/**
 * [p3] §31.7's readout: `<lit>/<evaluable>`, and `null` when either is NULL.
 *
 * **Never a percentage and never a bare numerator.** The denominator is the whole safeguard: a
 * fraction whose denominator is the truth makes no claim of ten, and a percentage erases it.
 * `null` means *render no node*, which is `uncomputedRank`'s rule one column over.
 */
export function scoreText(lit: number | null, evaluable: number | null): string | null {
  if (lit === null || evaluable === null || evaluable <= 0) return null;
  return `${lit}/${evaluable}`;
}

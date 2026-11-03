/**
 * §40.9: the one module that says what a badge award reads as. §43's badge line and its earned
 * panel, and §42's Profile and wrap, import it and never restate it.
 *
 * Every string is interim: its constraint text, until the user writes the words. A clause that
 * names a word the constraint bans, or a threshold the user has not ruled copy may name, is left
 * out of the rendered string; each export's comment keeps the constraint whole.
 */
import type { BadgeAward, BadgeKind } from '../../generated/protocol';

/** What one award reads as: its display name and its one sentence. */
export interface BadgeAwardCopy {
  readonly name: string;
  readonly sentence: string;
}

/**
 * Every kind's display name. The constraint in full: a display name and one sentence per badge;
 * never the slug; never a line count, a commit count or a count toward a threshold; names a
 * threshold only if the user rules copy may; dates render `earnedOn`.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
const BADGE_AWARD_NAME =
  '‹COPY: a display name and one sentence per badge; never the slug; never a line count, a commit count or a count toward a threshold; names a threshold only if the user rules copy may; dates render earnedOn›';

/**
 * Each kind's sentence. The constraints in full, where the rendered interim drops a clause:
 *
 * - `polyglot`: names the year and the five languages, framed by each project's primary language.
 * - `comeback`: names the return date; whether it states the gap is the user's call; "welcome
 *   back" is allowed, "we missed you" is not; never frames the gap as a fault.
 * - `curator`: says "of what could be scored" whenever any listed project is notched; never
 *   "complete" or "10 of 10".
 * - `undertaker`: names the five projects; claims an observation, never "safe", "backed up" or
 *   "verified".
 * - `firefighter`: names the tenth pair's project and date; claims a red build turned green the
 *   day it went red, on a day you committed there — no more.
 * - `lifeline`: claims each project's checked-out branch was pushed as last observed, each day;
 *   never "verified", "backed up", "no unpushed work anywhere", or anything about other branches.
 */
// ‹COPY› interim — the user's words replace this before v1.0.0
const BADGE_AWARD_SENTENCE: Readonly<Record<BadgeKind, string>> = {
  necromancer:
    "‹COPY: names the project and the return date; may state gapDays; the RPG-tone pass is the user's›",
  polyglot: "‹COPY: names the year and the languages, framed by each project's primary language›",
  comeback:
    "‹COPY: names the return date; whether it states the gap is the user's call; never frames the gap as a fault›",
  curator: '‹COPY: says of what could be scored whenever any listed project is notched›',
  marathon: "‹COPY: names the project and the credited time in PLAYTIME's one formatter›",
  undertaker: '‹COPY: names the projects; claims an observation›',
  firefighter:
    "‹COPY: names the pair's project and date; claims a red build turned green the day it went red, on a day you committed there — no more›",
  lifeline:
    "‹COPY: claims each project's checked-out branch was pushed as last observed, each day›",
};

/** `award`'s name and sentence. Neither renders the kind's slug, a threshold or a count. */
export function badgeAwardCopy(award: BadgeAward): BadgeAwardCopy {
  return { name: BADGE_AWARD_NAME, sentence: BADGE_AWARD_SENTENCE[award.kind] };
}

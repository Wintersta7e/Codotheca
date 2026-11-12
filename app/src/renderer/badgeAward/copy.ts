/**
 * §40.9: the one module that says what a badge award reads as. §43's badge line and its earned
 * panel, and §42's Profile and wrap, import it and never restate it.
 *
 * No string names the slug, a line count, a commit count or a count toward a threshold; a gap
 * (`gapDays`) is a fact about the award and may be stated. Dates render `earnedOn` as it comes,
 * and every evidence field is nullable: a sentence whose figure or name is missing drops that
 * clause or names the subject in general terms — never `0`, never an empty name.
 */
import type { BadgeAward, BadgeEvidence, BadgeKind } from '../../generated/protocol';
import { formatPlaytime } from '../format/playtime';

/** What one award reads as: its display name and its one sentence. */
export interface BadgeAwardCopy {
  readonly name: string;
  readonly sentence: string;
}

/** Every kind's display name. A name carries no evidence: §42's wrap shows kind and date only. */
const BADGE_AWARD_NAME: Readonly<Record<BadgeKind, string>> = {
  necromancer: 'REVIVED',
  polyglot: 'MANY LANGUAGES, ONE YEAR',
  comeback: 'BACK TO WORK',
  curator: 'GOLD SHELF',
  marathon: 'LONG SESSION',
  undertaker: 'CLEANLY ARCHIVED',
  firefighter: 'SAME-DAY FIX',
  lifeline: 'DAILY PUSH',
};

/** The most project names Curator lists before it says how many more; its list is unbounded. */
const CURATOR_NAMES_SHOWN = 3;

/** `A`, `A and B`, `A, B and C`: the one way a sentence here joins a list. */
export function joinList(items: readonly string[]): string {
  const last = items.at(-1);
  if (last === undefined) return '';
  return items.length === 1 ? last : `${items.slice(0, -1).join(', ')} and ${last}`;
}

/** The names a list field holds, empty ones dropped; `null` when none remain. */
function names(list: readonly string[] | null): readonly string[] | null {
  const kept = (list ?? []).filter((name) => name !== '');
  return kept.length > 0 ? kept : null;
}

/** The award's project name, or `null` when it carries none. */
function projectName(evidence: BadgeEvidence): string | null {
  return evidence.projectName === null || evidence.projectName === '' ? null : evidence.projectName;
}

type Sentence = (evidence: BadgeEvidence, earnedOn: string) => string;

/**
 * Each kind's sentence. The claims each one may make, from §40.9:
 *
 * - `polyglot` frames the languages as the projects' main ones, never what you wrote.
 * - `comeback` is a return to work, never to the app; "welcome back" is allowed, never a fault.
 * - `curator` says "could be scored" always — true of plain gold and of notched gold alike.
 * - `undertaker` and `lifeline` claim an observation, never "safe", "backed up" or "verified".
 * - `firefighter` claims a red build turned green the day it went red, on a commit-day — no more.
 */
const BADGE_AWARD_SENTENCE: Readonly<Record<BadgeKind, Sentence>> = {
  necromancer: (evidence, earnedOn) => {
    const gap = evidence.gapDays === null ? '' : `, after ${String(evidence.gapDays)} days dormant`;
    return `${projectName(evidence) ?? 'A project'} had a commit again on ${earnedOn}${gap}.`;
  },
  polyglot: (evidence) => {
    const year = evidence.year === null ? 'In one year' : `In ${String(evidence.year)}`;
    const languages = names(evidence.languages);
    return languages === null
      ? `${year} you committed to projects in several main languages.`
      : `${year} you committed to projects whose main languages were ${joinList(languages)}.`;
  },
  comeback: (evidence, earnedOn) =>
    evidence.gapDays === null
      ? `Welcome back. Work picked up again on ${earnedOn}.`
      : `Welcome back. Work picked up again on ${earnedOn}, ` +
        `${String(evidence.gapDays)} days after the commit-day before it.`,
  curator: (evidence, earnedOn) => {
    const listed = names(evidence.projects?.map((project) => project.name) ?? null);
    let subject = 'Your projects';
    if (listed !== null) {
      const more = listed.length - CURATOR_NAMES_SHOWN;
      subject =
        more > 0
          ? `${listed.slice(0, CURATOR_NAMES_SHOWN).join(', ')} and ${String(more)} more`
          : joinList(listed);
    }
    return `${subject} each passed every check that could be scored, as observed on ${earnedOn}.`;
  },
  marathon: (evidence, earnedOn) => {
    const project = projectName(evidence) ?? 'a project';
    return evidence.creditedSeconds === null
      ? `One long session of ${project}, ending ${earnedOn}.`
      : `One session of ${project} counted ${formatPlaytime(evidence.creditedSeconds)} of ` +
          `playtime, ending ${earnedOn}.`;
  },
  undertaker: (evidence) => {
    const listed = names(evidence.projects?.map((project) => project.name) ?? null);
    return `${listed === null ? 'Projects on your shelf' : joinList(listed)} were each archived cleanly, as last observed.`;
  },
  firefighter: (evidence, earnedOn) => {
    const name = projectName(evidence);
    return (
      `On ${earnedOn}, ${name === null ? "a project's" : `${name}'s`} build went red and turned ` +
      'green the same day, a day you committed there.'
    );
  },
  lifeline: (evidence, earnedOn) => {
    const span =
      evidence.runStartOn === null || evidence.runStartOn === ''
        ? `Up to ${earnedOn}`
        : `From ${evidence.runStartOn} to ${earnedOn}`;
    return `${span}, each project's checked-out branch was pushed as last observed, every day.`;
  },
};

/** `award`'s name and sentence. Neither renders the kind's slug, a threshold or a count. */
export function badgeAwardCopy(award: BadgeAward): BadgeAwardCopy {
  return {
    name: BADGE_AWARD_NAME[award.kind],
    sentence: BADGE_AWARD_SENTENCE[award.kind](award.evidence, award.earnedOn),
  };
}

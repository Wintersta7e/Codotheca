import { COVERAGE_HISTORY_GROWS, COVERAGE_HISTORY_MOVES, COVERAGE_PARTIAL } from './copy';
import type { ProjectId, Reveal, RevealBasis } from '../../generated/protocol';

export type RevealKey =
  | 'spanDays'
  | 'projectCount'
  | 'languageCount'
  | 'bestYear'
  | 'oldestStillAlive'
  | 'playtimeSeconds';

/** §10.4a: the design's eight minus LEVEL and BADGE EARNED, both phase 4. */
export const REVEAL_ORDER: readonly RevealKey[] = [
  'spanDays',
  'projectCount',
  'languageCount',
  'bestYear',
  'oldestStillAlive',
  'playtimeSeconds',
];

const LABELS: Readonly<Record<RevealKey, string>> = {
  spanDays: 'SPAN',
  projectCount: 'PROJECTS',
  languageCount: 'LANGUAGES',
  bestYear: 'BEST YEAR',
  oldestStillAlive: 'OLDEST STILL ALIVE',
  playtimeSeconds: 'PLAYTIME',
};

export interface RevealProject {
  readonly name: string;
  readonly birthYear: number | null;
  readonly primaryLanguage: string | null;
}

export interface LanguageCount {
  readonly name: string;
  readonly count: number;
}

/**
 * What the shelf's own rows can tell the reveal that `Reveal` does not carry. All four are
 * derivable from the `ProjectRow[]` the renderer already holds, so none of them is a second
 * call and none can disagree with a figure.
 */
export interface RevealDeps {
  /** R3: epoch seconds, handed in. This module never reads a clock. */
  readonly nowSecs: number;
  readonly languageTally: readonly LanguageCount[];
  readonly referenceCount: number | null;
  readonly project: (id: ProjectId) => RevealProject | null;
}

export interface RevealPanel {
  readonly key: RevealKey;
  readonly label: string;
  /** `null` means *not computed*. It is never rendered as `0` and never as `—`. */
  readonly value: string | null;
  /** §10.4a: 22px for OLDEST STILL ALIVE, 34px for the rest. */
  readonly wide: boolean;
  readonly coverage: string | null;
  readonly caption: string;
  readonly evidence: string;
  /** §10.4a: panel 1 takes --sig; panels 2–6 take --line-4. The gold edge is phase 4. */
  readonly signalEdge: boolean;
  readonly delaySec: number;
}

/** §10.4a: `0.06 + i × 0.07` — the reveal's own step, not the 80 ms generic cascade. */
export const PANEL_ENTRY_FIRST_SEC = 0.06;
export const PANEL_ENTRY_STEP_SEC = 0.07;

const DAYS_PER_YEAR = 365.2425;

/**
 * §10.4a. Span is fractional days so the exact date of the earliest commit is recoverable
 * without an off-by-one at a new year's boundary.
 *
 * GAP-16b-3: §10.4a's headline ladder has no sub-year rung, so a span under one full year
 * returns `null` rather than printing `AT LEAST 0 YEARS`.
 */
export function spanYears(spanDays: number | null): number | null {
  if (spanDays === null) return null;
  const years = Math.floor(spanDays / DAYS_PER_YEAR);
  return years >= 1 ? years : null;
}

/** §10.4a's two strings, and which panels take which. */
export function coverageLine(key: RevealKey, basis: RevealBasis): string | null {
  // 0h is complete by construction, and it is the only figure on the screen that is.
  if (key === 'playtimeSeconds') return null;
  const parts: string[] = [];
  if (basis.projectsCovered < basis.projectsTotal) {
    parts.push(COVERAGE_PARTIAL(basis.projectsCovered));
  }
  if (!basis.historyComplete) {
    // A history still arriving can move BEST YEAR to a different year and OLDEST STILL ALIVE to
    // a different project; span can only grow.
    if (key === 'spanDays') parts.push(COVERAGE_HISTORY_GROWS);
    if (key === 'bestYear' || key === 'oldestStillAlive') parts.push(COVERAGE_HISTORY_MOVES);
  }
  return parts.length === 0 ? null : parts.join(' · ');
}

function basisOf(key: RevealKey, reveal: Reveal): RevealBasis {
  if (key === 'oldestStillAlive') return reveal.oldestStillAlive.basis;
  return reveal[key].basis;
}

function earliestYear(reveal: Reveal, deps: RevealDeps): number | null {
  const days = reveal.spanDays.value;
  if (days === null) return null;
  return new Date((deps.nowSecs - days * 86_400) * 1000).getUTCFullYear();
}

export function panelFor(key: RevealKey, reveal: Reveal, deps: RevealDeps): RevealPanel {
  const basis = basisOf(key, reveal);
  const shared = {
    key,
    label: LABELS[key],
    coverage: coverageLine(key, basis),
    signalEdge: key === 'spanDays',
    delaySec: PANEL_ENTRY_FIRST_SEC + REVEAL_ORDER.indexOf(key) * PANEL_ENTRY_STEP_SEC,
    wide: key === 'oldestStillAlive',
  } as const;

  switch (key) {
    case 'spanDays': {
      const years = spanYears(reveal.spanDays.value);
      const born = earliestYear(reveal, deps);
      return {
        ...shared,
        value: years === null ? null : `${String(years)} ${years === 1 ? 'YEAR' : 'YEARS'}`,
        caption:
          born === null
            ? 'Nothing else on your machine knows this.'
            : `First commit ${String(born)}. Nothing else on your machine knows this.`,
        evidence:
          born === null
            ? 'Shallow clones are excluded from the span and counted in the coverage line.'
            : `Earliest first commit: ${String(born)}\nCounted inclusively · shallow clones excluded and counted in the coverage line.`,
      };
    }
    case 'projectCount': {
      const total = reveal.projectCount.value;
      const refs = deps.referenceCount;
      let caption: string;
      if (refs === null) caption = 'Everything indexed so far.';
      else if (refs > 0) {
        caption = `${String(refs)} of them are other people's code, and are left out of every judgement.`;
      } else caption = "All of them yours. Nothing here is someone else's code you cloned once.";
      return {
        ...shared,
        value: total === null ? null : String(total),
        caption,
        evidence: `${String(basis.projectsCovered)} of ${String(basis.projectsTotal)} indexed`,
      };
    }
    case 'languageCount': {
      const names = deps.languageTally.map((l) => l.name);
      return {
        ...shared,
        value: reveal.languageCount.value === null ? null : String(reveal.languageCount.value),
        caption: names.length === 0 ? 'Classified from what is on disk.' : `${names.join(', ')}.`,
        evidence: deps.languageTally.map((l) => `${l.name} ${String(l.count)}`).join('  ·  '),
      };
    }
    case 'bestYear': {
      const year = reveal.bestYear.value;
      return {
        ...shared,
        value: year === null ? null : String(year),
        caption: 'Your densest year of starts.',
        evidence:
          year === null
            ? 'No full year of starts is covered yet.'
            : `${String(year)} · the year in which the most projects had their first commit · shallow clones excluded`,
      };
    }
    case 'oldestStillAlive': {
      const id = reveal.oldestStillAlive.projectId;
      const project = id === null ? null : deps.project(id);
      const born = project?.birthYear ?? null;
      const language = project?.primaryLanguage ?? null;
      return {
        ...shared,
        value: project?.name ?? null,
        // §10.4a: `Still builds.` is a build claim and phase 1 builds nothing.
        caption: 'Still yours.',
        evidence:
          project === null
            ? 'No history has returned yet.'
            : [project.name, born === null ? null : `born ${String(born)}`, language]
                .filter((part): part is string => part !== null)
                .join('  ·  '),
      };
    }
    case 'playtimeSeconds': {
      const seconds = reveal.playtimeSeconds.value ?? 0;
      return {
        ...shared,
        value: `${String(Math.floor(seconds / 3600))}h`,
        caption: 'Starts now. This is the one number the app cannot know about your past.',
        evidence:
          'Launched-session time only. Nothing before this install can be counted, and no estimate is offered.',
      };
    }
  }
}

export function revealPanels(reveal: Reveal, deps: RevealDeps): readonly RevealPanel[] {
  return REVEAL_ORDER.map((key) => panelFor(key, reveal, deps));
}

/**
 * §10.3a: each milestone renders one reveal figure computed over what is indexed at that
 * instant. The figures are these panels, from the same `stats.reveal` call, so a milestone and
 * the reveal cannot disagree by construction.
 */
export const MILESTONE_FIGURES: Readonly<Record<number, RevealKey>> = {
  10: 'languageCount',
  50: 'spanDays',
  100: 'oldestStillAlive',
  250: 'bestYear',
  500: 'spanDays',
};

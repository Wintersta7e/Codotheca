/**
 * §43.7's two reveal panels, `LEVEL` and `BADGE EARNED`, and where they sit among §10.4a's six.
 *
 * They are separate from `revealPanels` because they read different calls: `LEVEL` reads the one
 * motivation read and `BADGE EARNED` reads `badges.list`, never `stats.reveal` — the level and the
 * badges are on quiet's list and the reveal figures are not, so one call must not carry both.
 * Under quiet both are absent, and the reveal is §10.4a's six.
 */
import type {
  BadgeKind,
  BadgesListed,
  MotivationState,
  XpTermTotal,
} from '../../generated/protocol';
import type { QuietEntryId } from '../../shared/motivationQuiet';
import { badgeAwardCopy } from '../badgeAward/copy';
import {
  BADGE_EARNED_LABEL,
  FLOOR_HOLDS_LINE,
  LEVEL_LABEL,
  LEVEL_PANEL_CAPTION,
  TERMS_SUM_LABEL,
  XP_TERM_WORDS,
} from '../motivation/copy';
import { COVERAGE_HISTORY_GROWS, UNCOMPUTED_NOTE } from './copy';
import { PANEL_ENTRY_FIRST_SEC, PANEL_ENTRY_STEP_SEC } from './revealModel';
import type { RevealKey, RevealPanel } from './revealModel';

export type RevealMotivationKey = 'level' | 'badgeEarned';

export interface RevealMotivationPanel {
  readonly key: RevealMotivationKey;
  readonly label: string;
  /** `null` means *not computed*: never `0`, never `1`. */
  readonly value: string | null;
  /** §10.4a's 22px name size: a badge's name is a word, not a figure. */
  readonly wide: boolean;
  readonly coverage: string | null;
  readonly caption: string;
  readonly evidence: string;
  /** What `revealLevelShown` carries: the level displayed, or `null` when it is not computed. */
  readonly shownLevel: number | null;
  /** The award to land once displayed — `null` when none is drawn or it has already landed. */
  readonly toLand: BadgeKind | null;
}

/** A panel as the reveal draws it, whichever call it came from. */
export interface RevealDrawnPanel {
  readonly key: RevealKey | RevealMotivationKey;
  readonly label: string;
  readonly value: string | null;
  readonly wide: boolean;
  readonly coverage: string | null;
  readonly caption: string;
  readonly evidence: string;
  /** §10.4a: panel 1 takes `--sig`; §43.7: `BADGE EARNED` takes `--tier-gold`, the only gold. */
  readonly edge: 'sig' | 'gold' | null;
  /** §10.4a's `0.06 + i × 0.07` s, by position in what is drawn. */
  readonly delaySec: number;
  /** The quiet entry the panel belongs to, or `null` for a reveal figure quiet keeps. */
  readonly quiet: QuietEntryId | null;
}

/** §43.13's evidence: each term by its own name and figure, then their sum, then the floor. */
function levelEvidence(terms: readonly XpTermTotal[] | null, floored: boolean): string {
  const lines =
    terms === null
      ? []
      : [
          ...terms.map((entry) => `${XP_TERM_WORDS[entry.term]}  ${String(entry.xp)}`),
          `${TERMS_SUM_LABEL}  ${String(terms.reduce((total, entry) => total + entry.xp, 0))}`,
        ];
  if (floored) lines.push(FLOOR_HOLDS_LINE);
  return lines.join('\n');
}

/** A state the schema added after this table: fail rather than draw it blank. */
function noRendering(state: never): never {
  throw new Error(`the reveal's LEVEL panel has no rendering for ${String(state)}`);
}

/**
 * §43.7's `LEVEL`: §10.4's contract — value, coverage, refreshed in place — over `motivation.get`.
 * `null` under quiet.
 */
export function levelPanel(state: MotivationState): RevealMotivationPanel | null {
  if (!state.shown) return null;
  const shared = {
    key: 'level',
    label: LEVEL_LABEL,
    wide: false,
    caption: LEVEL_PANEL_CAPTION,
    toLand: null,
  } as const;
  const uncomputed: RevealMotivationPanel = {
    ...shared,
    value: null,
    coverage: null,
    evidence: UNCOMPUTED_NOTE,
    shownLevel: null,
  };
  const reading = state.level;
  if (reading === null) return uncomputed;
  const figures = reading.figures;
  const kind = reading.state;
  switch (kind) {
    case 'notComputed':
      return uncomputed;
    case 'partial':
    case 'computed': {
      if (figures === null) return uncomputed;
      return {
        ...shared,
        value: String(figures.level),
        // §43.7: true while partial — added history only adds rows, and the level never falls.
        // A computed level is complete by construction and carries none.
        coverage: kind === 'partial' ? COVERAGE_HISTORY_GROWS : null,
        evidence: levelEvidence(state.terms, figures.floored),
        shownLevel: figures.level,
      };
    }
    default:
      return noRendering(kind);
  }
}

/**
 * §43.7's `BADGE EARNED`, from §40's answer. `due` names a kind, so the award is looked up in
 * `awards`. `null` under quiet, before `badges.list` answers, and over complete history with none
 * due — a known absence in a celebration is not drawn.
 */
export function badgePanel(
  listed: BadgesListed | null,
  shown: boolean,
): RevealMotivationPanel | null {
  if (!shown || listed === null) return null;
  const due = listed.due;
  const award = due === null ? undefined : listed.awards.find((entry) => entry.kind === due);
  if (award !== undefined) {
    const copy = badgeAwardCopy(award);
    return {
      key: 'badgeEarned',
      label: BADGE_EARNED_LABEL,
      value: copy.name,
      wide: true,
      // A latched badge is a fact, not a measurement: no coverage row.
      coverage: null,
      caption: copy.sentence,
      evidence: award.earnedOn,
      shownLevel: null,
      toLand: award.landed ? null : award.kind,
    };
  }
  // A `due` the awards do not hold is not an answer either: it stays *not computed*.
  if (due === null && listed.basis.historyComplete) return null;
  return {
    key: 'badgeEarned',
    label: BADGE_EARNED_LABEL,
    value: null,
    wide: false,
    coverage: null,
    caption: '',
    evidence: UNCOMPUTED_NOTE,
    shownLevel: null,
    toLand: null,
  };
}

/** True while `BADGE EARNED` is drawn *not computed*, which is when it is refreshed. */
export function badgePending(listed: BadgesListed): boolean {
  return badgePanel(listed, true)?.value === null;
}

/**
 * §43.7's floor follows the count: `260px` fits three tracks of the 860px column (3 + 3 at six);
 * `200px` fits four (4 + 3 at seven, 4 + 4 at eight). At eight under `260px` the reveal reads
 * 3 + 3 + 2, the failure §10.4a's own argument names.
 */
export function revealGridFloorPx(panelCount: number): 200 | 260 {
  return panelCount >= 7 ? 200 : 260;
}

function drawnFrom(panel: RevealMotivationPanel): Omit<RevealDrawnPanel, 'delaySec'> {
  return {
    key: panel.key,
    label: panel.label,
    value: panel.value,
    wide: panel.wide,
    coverage: panel.coverage,
    caption: panel.caption,
    evidence: panel.evidence,
    edge: panel.key === 'badgeEarned' ? 'gold' : null,
    quiet: panel.key === 'badgeEarned' ? 'badges' : 'level',
  };
}

/**
 * §43.7's order — the five, `LEVEL`, `PLAYTIME`, `BADGE EARNED` — with each delay by position, so
 * an absent panel leaves no gap in the cascade.
 */
export function revealSequence(
  six: readonly RevealPanel[],
  level: RevealMotivationPanel | null,
  badge: RevealMotivationPanel | null,
): readonly RevealDrawnPanel[] {
  const fromReveal = (panel: RevealPanel): Omit<RevealDrawnPanel, 'delaySec'> => ({
    key: panel.key,
    label: panel.label,
    value: panel.value,
    wide: panel.wide,
    coverage: panel.coverage,
    caption: panel.caption,
    evidence: panel.evidence,
    edge: panel.signalEdge ? 'sig' : null,
    quiet: null,
  });
  const playtime = six.filter((panel) => panel.key === 'playtimeSeconds');
  const rest = six.filter((panel) => panel.key !== 'playtimeSeconds');
  const ordered = [
    ...rest.map(fromReveal),
    ...(level === null ? [] : [drawnFrom(level)]),
    ...playtime.map(fromReveal),
    ...(badge === null ? [] : [drawnFrom(badge)]),
  ];
  return ordered.map((panel, i) => ({
    ...panel,
    delaySec: PANEL_ENTRY_FIRST_SEC + i * PANEL_ENTRY_STEP_SEC,
  }));
}

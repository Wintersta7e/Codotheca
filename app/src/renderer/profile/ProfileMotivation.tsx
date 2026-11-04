/**
 * §43.13's two sections of Profile: the level with its evidence open, and every latched badge.
 * Under §10.5a's no-cascade rule — no panel cascade, no ceremony, no turn — and under quiet
 * neither is drawn. Every element quiet mode removes carries `data-motivation`.
 */
import type { ReactElement } from 'react';

import type { BadgesListed, MotivationState } from '../../generated/protocol';
import { DATA_MOTIVATION } from '../../shared/motivationQuiet';
import { badgeAwardCopy } from '../badgeAward/copy';
import {
  BADGES_LABEL,
  FLOOR_HOLDS_LINE,
  LEVEL_LABEL,
  LEVEL_NOT_COMPUTED,
  TERMS_SUM_LABEL,
  XP_TERM_WORDS,
} from '../motivation/copy';
import { levelBlockModel } from '../motivation/levelBlock';

const quiet = (id: string): Record<string, string> => ({ [DATA_MOTIVATION]: id });

/** The attribute naming a section, or a badge, as a place Profile can open at. */
export const PROFILE_ANCHOR = 'data-profile-anchor';

/**
 * §43.2's level table, then the evidence: `MotivationState.terms` term by term, and their sum
 * printed beside it as a check against the level's XP. With no figures there is nothing to sum.
 */
export function ProfileLevelSection(props: {
  readonly state: MotivationState;
}): ReactElement | null {
  const { state } = props;
  if (!state.shown) return null;
  const model = levelBlockModel(state.level, state.xpBar);
  const figures = state.level?.figures ?? null;
  const terms = figures === null ? null : state.terms;
  const sum = terms === null ? null : terms.reduce((total, entry) => total + entry.xp, 0);

  return (
    <section className="cdt-profile-section" {...{ [PROFILE_ANCHOR]: 'level' }} {...quiet('level')}>
      <h2 className="cdt-profile-heading">{LEVEL_LABEL}</h2>
      <div className="cdt-profile-level">
        <span
          className="cdt-profile-level-figure"
          data-unknown={model.unknown ? 'true' : undefined}
        >
          {model.number}
        </span>
        <span className="cdt-profile-line" {...quiet('xp')}>
          {model.line}
        </span>
      </div>
      {terms === null || sum === null ? null : (
        <table className="cdt-profile-terms" {...quiet('xp')}>
          <tbody>
            {terms.map((entry) => (
              <tr key={entry.term} data-term={entry.term}>
                <th scope="row">{XP_TERM_WORDS[entry.term]}</th>
                <td data-xp="">{entry.xp}</td>
              </tr>
            ))}
          </tbody>
          <tfoot>
            <tr>
              <th scope="row">{TERMS_SUM_LABEL}</th>
              <td data-sum="">{sum}</td>
            </tr>
          </tfoot>
        </table>
      )}
      {figures?.floored === true ? <p className="cdt-profile-line">{FLOOR_HOLDS_LINE}</p> : null}
    </section>
  );
}

/**
 * §40's latched badges in `badges.list`'s order, each dated `earnedOn`, with its name and its
 * evidence sentence — never a progress, a count, a meter or a silhouette. History unread and none
 * latched is *not computed*; history read and none latched draws no section at all.
 */
export function ProfileBadgesSection(props: {
  readonly listed: BadgesListed | null;
  readonly shown: boolean;
}): ReactElement | null {
  const { listed, shown } = props;
  if (!shown || listed === null) return null;
  if (listed.awards.length === 0 && listed.basis.historyComplete) return null;

  return (
    <section
      className="cdt-profile-section"
      {...{ [PROFILE_ANCHOR]: 'badges' }}
      {...quiet('badges')}
    >
      <h2 className="cdt-profile-heading">{BADGES_LABEL}</h2>
      {listed.awards.length === 0 ? (
        <p className="cdt-profile-line">{LEVEL_NOT_COMPUTED}</p>
      ) : (
        <ul className="cdt-profile-badges">
          {listed.awards.map((award) => {
            const copy = badgeAwardCopy(award);
            return (
              <li
                key={`${award.kind}:${award.earnedOn}`}
                className="cdt-profile-badge"
                {...{ [PROFILE_ANCHOR]: `badge:${award.kind}` }}
              >
                <span className="cdt-profile-badge-name">{copy.name}</span>
                <span className="cdt-profile-line">{copy.sentence}</span>
                <time className="cdt-profile-line" dateTime={award.earnedOn}>
                  {award.earnedOn}
                </time>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

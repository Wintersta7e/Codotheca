/**
 * §43.4's line, as a pure model: what the one waiting line says, which badge painting it lands,
 * and where its click opens Profile. Drawn from `MotivationState.pendingLine` and nothing else —
 * the line is derived from stored state, so the renderer holds none of it.
 */
import type {
  BadgeKind,
  LevelReading,
  MotivationLine,
  MotivationLineForm,
} from '../../generated/protocol';
import type { ProfileAnchor } from '../app/route';
import { badgeAwardCopy } from '../badgeAward/copy';
import { LINE_ARRIVAL, LINE_COVERAGE_PARTIAL, LINE_RETURN, LINE_RISE, UNKNOWN } from './copy';

export interface LineModel {
  readonly form: MotivationLineForm;
  /** The level forms' one line; a badge's display name. */
  readonly text: string;
  /** A badge's one evidence sentence (§40.9); `null` for the level forms. */
  readonly sentence: string | null;
  /**
   * The award painting this line lands (§40): its kind and the date it was earned, while it is
   * not yet landed. `null` for a level line and for a badge already landed.
   */
  readonly toLand: { readonly kind: BadgeKind; readonly earnedOn: string } | null;
  /** Where the click opens Profile. */
  readonly anchor: ProfileAnchor;
}

const level = (value: number | null): string => (value === null ? UNKNOWN : String(value));

/** A form the schema added after this table: fail rather than draw it blank. */
function noRendering(form: never): never {
  throw new Error(`the line has no rendering for ${String(form)}`);
}

/** `reading` is the window's `LevelReading`; it decides the arrival's coverage qualifier. */
export function lineModel(line: MotivationLine, reading: LevelReading | null): LineModel {
  const form = line.form;
  const atLevel: ProfileAnchor = { section: 'level' };
  switch (form) {
    case 'arrival': {
      const head = `${LINE_ARRIVAL} ${level(line.level)}`;
      const text =
        reading?.state === 'partial'
          ? `${head} · ${LINE_COVERAGE_PARTIAL(reading.pendingProjects)}`
          : head;
      return { form, text, sentence: null, toLand: null, anchor: atLevel };
    }
    case 'rise':
      return {
        form,
        text: LINE_RISE(line.level, line.reason),
        sentence: null,
        toLand: null,
        anchor: atLevel,
      };
    case 'return':
      return {
        form,
        text: `${LINE_RETURN} ${level(line.level)}`,
        sentence: null,
        toLand: null,
        anchor: atLevel,
      };
    case 'badge': {
      const award = line.award;
      // A badge line the core sent without its award names nothing, so it lands nothing.
      if (award === null) {
        return { form, text: UNKNOWN, sentence: null, toLand: null, anchor: { section: 'badges' } };
      }
      const copy = badgeAwardCopy(award);
      return {
        form,
        text: copy.name,
        sentence: copy.sentence,
        toLand: award.landed ? null : { kind: award.kind, earnedOn: award.earnedOn },
        anchor: { section: 'badges', badge: award.kind },
      };
    }
    default:
      return noRendering(form);
  }
}

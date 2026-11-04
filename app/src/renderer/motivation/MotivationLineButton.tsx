/**
 * §43.4's one persistent line: a `<button>` in the level block after the XP bar. It is not a live
 * region and never an alert — it waits to be found, and it never clears itself.
 *
 * A badge lands when its line is painted (§40): the first time an unlanded award's line is on
 * screen in an active window, it sends `badges.markLanded` once. The click lands nothing; it
 * acknowledges the line and opens Profile where the line points.
 */
import { type ReactElement, useEffect, useRef } from 'react';

import type { LevelReading, MotivationAck, MotivationLine } from '../../generated/protocol';
import { DATA_MOTIVATION } from '../../shared/motivationQuiet';
import type { AppDeps } from '../app/deps';
import type { ProfileAnchor } from '../app/route';
import { lineModel } from './line';

export interface MotivationLineButtonProps {
  readonly line: MotivationLine;
  readonly level: LevelReading | null;
  /** On screen in an active window — §43.9's *shown*, the only time a line counts as painted. */
  readonly onScreen: boolean;
  readonly request: AppDeps['request'];
  readonly onOpenProfile: (at: ProfileAnchor) => void;
}

const quietLine = { [DATA_MOTIVATION]: 'line' };

export function MotivationLineButton(props: MotivationLineButtonProps): ReactElement {
  const { line, level, onScreen, request, onOpenProfile } = props;
  const model = lineModel(line, level);
  const landKind = model.toLand?.kind ?? null;
  const landKey = model.toLand === null ? null : `${model.toLand.kind}:${model.toLand.earnedOn}`;
  // The award this mount has already asked the core to land, so a re-render while the core's
  // answer is still on its way sends nothing more.
  const landing = useRef<string | null>(null);

  useEffect(() => {
    if (!onScreen || landKind === null || landKey === null || landing.current === landKey) return;
    landing.current = landKey;
    request('badges.markLanded', { kind: landKind }).then(
      () => undefined,
      // A refused landing is asked again the next time the line is painted.
      () => {
        if (landing.current === landKey) landing.current = null;
      },
    );
  }, [onScreen, landKind, landKey, request]);

  const onClick = (): void => {
    const event: MotivationAck = model.form === 'badge' ? 'badgeLineClicked' : 'levelLineClicked';
    const shownLevel = model.form === 'badge' ? null : line.level;
    void request('motivation.acknowledge', { event, level: shownLevel }).then(
      () => undefined,
      () => undefined,
    );
    onOpenProfile(model.anchor);
  };

  return (
    <button
      type="button"
      className="cdt-band-line"
      data-form={model.form}
      onClick={onClick}
      {...quietLine}
    >
      <span className="cdt-band-line-text">{model.text}</span>
      {model.sentence === null ? null : (
        <span className="cdt-band-line-sentence">{model.sentence}</span>
      )}
    </button>
  );
}

import { type ReactElement, useEffect, useState } from 'react';
import type { InstallPreview, ProjectId, ProjectRow, SessionRef } from '../../generated/protocol';
import { CARD_ROLE } from '../a11y/names';
import { appearanceFor, fadeFor, languageCode, seedOf } from '../art/appearance';
import { renditionFor, useArtAddress, useCardBitmap } from '../art/useCardBitmap';
import { InstallControl } from '../install/InstallControl';
import { glowShadow, glowStrength } from '../derive/condition';
import { formatTrackedBytes } from '../format/size';
import type { CardGesture } from '../motion/transition';
import { Card } from './Card';
import { statusChips } from './chips';
import { frameToken, rungFor, uncomputedRank } from './completion';
import { densityStep } from './geometry';
import { BENCH_LABEL, useBenchElapsed } from './useBenchElapsed';
import { noop } from '../noop';

/**
 * The grid tile the shelf mounts, and the composition every value in the card modules was
 * derived for. It computes nothing of its own: appearance, chips, rank, glow, geometry and names
 * all arrive already decided, and this component chooses only which of them are on screen at
 * this density.
 *
 * Hover is held here, not on the shelf. §7.8's `hov = <projectId>` is one value seen from the
 * other end; a shelf-level id re-renders 140 mounted cards on every pointer move, and the
 * measured frame budget is 4.2 ms.
 *
 * Five props exceed the eight-field contract the shelf plan quotes, each forced by a spec
 * sentence: the first-run boundary `NEW` needs, the open session §7.8's live tile needs, the pin
 * and stop callbacks §7.8a and §7.8 need — the card may not call the protocol — and the flicker
 * dip, which is chosen above the card because "at most one card in the viewport" is a
 * shelf-level fact.
 */
export interface ProjectCardProps {
  readonly row: ProjectRow;
  /** The `--tile` value in px, §8.0a's three steps. */
  readonly density: number;
  readonly rendition: 'card';
  readonly selected: boolean;
  readonly focused: boolean;
  /**
   * Unix **seconds** (R3), and the shelf's shared instant for this paint. §11.7's as-of clauses
   * and §7.8's bench figure both re-derive from it, so a shelf that never advances it freezes
   * both. The card's own timer repaints at the minute boundary; it cannot invent a later second.
   */
  readonly now: number;
  readonly firstRunCompletedAt: number | null;
  readonly session: SessionRef | null;
  readonly haloOpacity: number;
  /** §8.5.1's gesture for this one tile, `null` when the shelf is at rest. */
  readonly gesture?: CardGesture | null;
  /** The ripple's own offset, already formatted (`motion/transition.ts` owns the arithmetic). */
  readonly gestureDelay?: string;
  readonly onActivate: () => void;
  readonly onOpen: () => void;
  readonly onTogglePin: () => void;
  readonly onStopSession: () => void;
  /**
   * [p2] §24.3d's preview for this project, or `undefined` when the shelf is not offering
   * Install here at all. **`undefined` is not the same as `null`**: `null` is *not computed yet*
   * and renders nothing, while absent means this surface never asks.
   */
  readonly installPreview?: InstallPreview | null | undefined;
  readonly onInstall?: (() => void) | undefined;
  readonly onOpenUpgrade?: (() => void) | undefined;
  /**
   * [p2] **The request is the demand** (§7.6), one surface down from the art address above. A
   * tile is mounted only while it is near the viewport, so asking here is what keeps the shelf
   * from previewing every not-cloned project in the library on first paint. The shelf owns the
   * answers; this only says which one is wanted.
   */
  readonly onNeedInstallPreview?: ((id: ProjectId) => void) | undefined;
}

export function ProjectCard(props: ProjectCardProps): ReactElement {
  const { row } = props;
  const [hovered, setHovered] = useState(false);
  const step = densityStep(props.density);
  const appearance = appearanceFor(seedOf(row), fadeFor(row), row.primaryLanguage);
  // §23.5: a project with no working copy asks for the second pass, at its own address. The
  // predicate is §23.1's one and only — `primaryLocation !== null`.
  const hasWorkingCopy = row.primaryLocation !== null;
  // [p3] §31.1c: `pct = lit / evaluable` and nothing else. `null` is one of the three cases
  // decided above the ladder, and `uncomputedRank` below draws that one.
  const rung = rungFor({
    completionLit: row.completionLit,
    completionApplicable: row.completionApplicable,
    isReference: row.isReference,
    hasWorkingCopy,
    isArchived: row.isArchived,
  });
  const rendition = renditionFor(props.rendition, hasWorkingCopy);
  const { onNeedInstallPreview } = props;
  useEffect(() => {
    if (hasWorkingCopy) return;
    onNeedInstallPreview?.(row.id);
  }, [hasWorkingCopy, onNeedInstallPreview, row.id]);
  // **The request is the demand** (§7.6), and the blueprint needs it. J5 writes the `card`
  // rendition and nothing else, so a located tile composes its address and the file is already
  // on disk — that path is untouched, and `null` here issues no command. A `card-blueprint` has
  // no writer but `art.url` itself, so composing its address without asking gets a 404 from the
  // shell and §7.5's plate, for ever. That is the hero's rule, one surface down.
  const demanded = useArtAddress(hasWorkingCopy ? null : row.artSceneHash, row.artState, rendition);
  const bitmap = useCardBitmap({
    sceneHash: row.artSceneHash,
    rendition,
    artState: row.artState,
    ...(hasWorkingCopy ? {} : { src: demanded }),
  });
  const bench = useBenchElapsed(
    props.session?.endedAt !== null ? null : props.session.startedAt,
    () => props.now,
  );

  const glow = glowShadow(
    glowStrength({
      signal: row.conditionSignal,
      isReference: row.isReference,
      isArchived: row.isArchived,
      hasOpenSession: bench !== null,
    }),
  );

  // §7.7's identity line: `<first-commit year> · <primary language>`, `owner` prefixed only when
  // it is not the user — which the projection expresses by leaving `owner` NULL when it is.
  // Whichever half is NULL is omitted; both NULL renders nothing at all.
  const identity = [
    row.owner,
    row.birthYear === null ? null : String(row.birthYear),
    row.primaryLanguage,
  ]
    .filter((part): part is string => part !== null && part !== '')
    .join(' · ');

  return (
    <Card
      surface="card"
      appearance={appearance}
      // [p3] §31.1c. `rungFor` answers `null` for the three cases decided ABOVE the ladder —
      // Reference, no working copy, and an uncomputed measurement — which is exactly where
      // §7.7a's own frame applies, so the two never both answer.
      frameToken={rung?.frameToken ?? frameToken({ isReference: row.isReference, hasWorkingCopy })}
      notched={rung?.notched ?? false}
      density={props.density}
      isArchived={row.isArchived}
      isReference={row.isReference}
      halo={{ shadow: glow, opacity: props.haloOpacity }}
      hovered={hovered}
      focused={props.focused}
      selected={props.selected}
      gesture={props.gesture ?? null}
      {...(props.gestureDelay === undefined ? {} : { gestureDelay: props.gestureDelay })}
      role={CARD_ROLE}
      tabIndex={props.focused ? 0 : -1}
      onMouseEnter={() => {
        setHovered(true);
      }}
      onMouseLeave={() => {
        setHovered(false);
      }}
      onClick={(shiftKey) => {
        if (shiftKey) props.onOpen();
        else props.onActivate();
      }}
      art={
        bitmap.src === null ? null : (
          <img
            className="cdt-art"
            alt=""
            src={bitmap.src}
            decoding="async"
            style={{
              position: 'absolute',
              inset: 0,
              width: '100%',
              height: '100%',
              display: 'block',
            }}
          />
        )
      }
      bands={{
        languageCode: row.primaryLanguage === null ? null : languageCode(row.primaryLanguage),
        designation: appearance.designation,
        hazard: row.conditionSignal === 'abandoned' && !row.isReference,
        conditionSignal: row.conditionSignal,
        rank: uncomputedRank('gridCard', {
          completionLit: row.completionLit,
          isReference: row.isReference,
          hasWorkingCopy,
          density: props.density,
        }),
        chips: statusChips(row, props.now, props.firstRunCompletedAt),
        pin: {
          projectName: row.name,
          isPinned: row.isPinned,
          surface: 'card',
          visible: row.isPinned || hovered || props.focused,
          onToggle: props.onTogglePin,
        },
      }}
    >
      <h3 className="cdt-name">{row.name}</h3>
      {step.showDescription && row.description !== null ? (
        <p className="cdt-desc">{row.description}</p>
      ) : null}
      {step.showIdentity && identity !== '' ? <p className="cdt-identity">{identity}</p> : null}
      {step.showStrip ? (
        <div className="cdt-strip" aria-hidden="true">
          {row.branch === null ? null : <span className="cdt-strip-branch">{row.branch}</span>}
          <span className="cdt-strip-rule" />
          {/* A NULL inventory is unmeasured, and an unmeasured size renders no figure — never a
              zero, which would read as an empty repository. */}
          {row.sizeTrackedBytes === null ? null : (
            <span>{formatTrackedBytes(row.sizeTrackedBytes)}</span>
          )}
        </div>
      ) : null}
      {/* [p2] §24.3d: Install sits in the slot Play occupies on a cloned project, which on a
          blueprint card is the card body itself. One of exactly two mount points in the whole
          renderer; `app/test/installSites.test.ts` fails on a third. */}
      {hasWorkingCopy || props.installPreview === undefined ? null : (
        <div
          className="cdt-card-install"
          onClick={(event) => {
            // The card body is Play; on a blueprint there is nothing to play, and the control
            // must not inherit a launch the project cannot perform.
            event.stopPropagation();
          }}
          role="presentation"
        >
          <InstallControl
            preview={props.installPreview}
            onInstall={props.onInstall ?? noop}
            onOpenUpgrade={props.onOpenUpgrade}
          />
        </div>
      )}
      {bench === null ? null : (
        <div className="cdt-bench">
          <span>{`${BENCH_LABEL} · ${bench}`}</span>
          {/* Stops propagation for the same reason the pin does: the card body is Play. Stopping
              a session writes no working tree and is not a §17 destructive operation. */}
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              props.onStopSession();
            }}
          >
            STOP
          </button>
        </div>
      )}
    </Card>
  );
}

import type { CSSProperties, ReactElement, ReactNode } from 'react';
import type { ConditionSignal } from '../../generated/protocol';
import type { CardAppearance } from '../art/appearance';
import type { CardGesture } from '../motion/transition';
import { type TokenName, token } from '../theme/tokens';
import { CardPlate } from './CardPlate';
import { ConditionDotMark } from './ConditionDotMark';
import { PinControl, type PinControlProps } from './PinControl';
import type { StatusChip } from './chips';
import { GOLD_NOTCH, type UncomputedRank } from './completion';
import { HAZARD_TAPE_HEIGHT_PX, type CardSurface, bandsFor, densityStep } from './geometry';
import { bloomShadow, cardCustomProperties } from './interaction';

/**
 * The card shell: three unclipped siblings, the 1px chamfered frame, and §7.7's five reserved
 * bands. **No element may occupy two bands** — four separate overlap bugs came from ignoring
 * them — and every edge here is read from `bandsFor(surface)` rather than remembered.
 *
 * Three things sit **outside** the clip, for one reason: `clip-path` deletes an outer
 * `box-shadow` and a negative offset exactly as it deletes an `outline`. `.cdt-card-halo` carries
 * §5.4a's glow and the flicker's dip, `.cdt-bloom` carries §7.8's hover bloom, and
 * `.cdt-frame-gap` carries §7.7a's hole in the frame's top edge at `top: -1px`.
 *
 * The whole custom-property bag is set on `.cdt-card-frame`, not on the card: `--cdt-bloom` is
 * read by the card's *sibling*, and custom properties inherit. `data-hovered` goes on both,
 * because `card.css` reaches the bloom through the frame and the other nine effects through the
 * card.
 */
export interface CardHalo {
  /** §5.4a's steady glow, already computed by the condition ladder. `null` draws none. */
  readonly shadow: string | null;
  /** §11.6's dip, `1` at steady. A value, never an inline `opacity`. */
  readonly opacity: number;
}

export interface CardBands {
  /** §7.3a's `LANG` code. `null` — an unmeasured language — renders no plate at all. */
  readonly languageCode: string | null;
  readonly designation: string;
  readonly hazard: boolean;
  readonly conditionSignal: ConditionSignal | null;
  /** `null` only once a later phase computes completion; in phase 1 every card has one. */
  readonly rank: UncomputedRank | null;
  readonly chips: readonly StatusChip[];
  /** §7.8a mounts the control on the grid tile only. */
  readonly pin: PinControlProps | null;
}

export interface CardProps {
  readonly surface: CardSurface;
  readonly appearance: CardAppearance;
  /**
   * [p3] §31.1c widened this from §7.7a's three above-the-ladder frames to **any** tier token,
   * because phase 3 is the first phase that paints a rung. `rungFor` names the token; nothing
   * here chooses one.
   */
  readonly frameToken: TokenName;
  /**
   * [p3] §31.1c's gold notch. It qualifies a 100% measurement taken over fewer than ten checks,
   * and it is **never drawn beside `bands.rank`'s gap** — the gap says there is no measurement
   * at all, so a card showing both would be saying both.
   */
  readonly notched?: boolean;
  readonly density: number;
  readonly isArchived: boolean;
  readonly isReference: boolean;
  readonly art: ReactNode;
  /** [p3] §33.4's layer stack, passed straight through. **No band gains an element.** */
  readonly decay?: ReactNode;
  readonly bands: CardBands;
  readonly halo: CardHalo;
  readonly hovered: boolean;
  readonly focused: boolean;
  readonly selected: boolean;
  /**
   * §8.5.1's gesture for this tile. It lands on the frame rather than the card because
   * `crtCollapse` and `cardUnfold` drive `filter: brightness`, and the card is clipped — the
   * frame is the unclipped box every other whole-tile effect already uses.
   */
  readonly gesture?: CardGesture | null;
  readonly gestureDelay?: string;
  readonly role?: string;
  readonly tabIndex?: 0 | -1;
  readonly onMouseEnter?: () => void;
  readonly onMouseLeave?: () => void;
  /** §7.8a: the card body is Play. `shiftKey` mirrors `Shift+Enter` — the project page. */
  readonly onClick?: (shiftKey: boolean) => void;
  /** Band 5. The hero's belongs to the project page; the tile's to `ProjectCard`. */
  readonly children: ReactNode;
}

export function Card(props: CardProps): ReactElement {
  const table = bandsFor(props.surface);
  const step = densityStep(props.density);
  const { bands } = props;
  const hovered = String(props.hovered);

  const frameStyle = {
    ...cardCustomProperties(props.appearance, token(props.frameToken)),
    '--cdt-bloom': bloomShadow(props.appearance),
    '--cdt-halo': props.halo.shadow ?? 'none',
    '--cdt-halo-opacity': String(props.halo.opacity),
    ...(props.gestureDelay === undefined ? {} : { '--cdt-ripple-delay': props.gestureDelay }),
  } as CSSProperties;

  return (
    <div
      className="cdt-card-frame"
      data-hovered={hovered}
      data-gesture={props.gesture ?? undefined}
      style={frameStyle}
    >
      <span className="cdt-card-halo" aria-hidden="true" />
      <span className="cdt-bloom" aria-hidden="true" />
      {props.notched !== true || bands.rank !== null ? null : (
        <span
          className="cdt-frame-notch"
          aria-hidden="true"
          style={
            {
              right: GOLD_NOTCH.right,
              top: GOLD_NOTCH.top,
              width: GOLD_NOTCH.width,
              height: GOLD_NOTCH.height,
              // The same `setProperty` route the gap below takes, and for the same reason.
              'background-color': token('surface-1'),
            } as CSSProperties
          }
        />
      )}
      {bands.rank === null ? null : (
        <span
          className="cdt-frame-gap"
          aria-hidden="true"
          style={
            {
              left: bands.rank.gap.left,
              top: bands.rank.gap.top,
              width: bands.rank.gap.width,
              height: bands.rank.gap.height,
              // A `var()` is not a colour to jsdom's parser and an assignment to `backgroundColor`
              // is dropped; `setProperty` is what React does for a hyphenated key, and it keeps
              // the token rather than a hex.
              'background-color': token(bands.rank.gap.fillToken),
            } as CSSProperties
          }
        />
      )}

      <div
        className="cdt-card"
        data-surface={props.surface}
        data-density={step.name}
        data-hovered={hovered}
        data-focused={String(props.focused)}
        data-selected={String(props.selected)}
        {...(props.role === undefined ? {} : { role: props.role })}
        {...(props.tabIndex === undefined ? {} : { tabIndex: props.tabIndex })}
        {...(props.selected ? { 'aria-selected': true } : {})}
        onMouseEnter={props.onMouseEnter}
        onMouseLeave={props.onMouseLeave}
        onClick={(event) => {
          props.onClick?.(event.shiftKey);
        }}
      >
        <CardPlate
          surface={props.surface}
          isArchived={props.isArchived}
          art={props.art}
          decay={props.decay}
        >
          {/* Band 1 — furniture only. */}
          {bands.hazard ? (
            <span
              className="cdt-hazard"
              aria-hidden="true"
              style={{ height: `${String(HAZARD_TAPE_HEIGHT_PX)}px` }}
            />
          ) : null}
          {bands.languageCode === null ? null : (
            <span
              className="cdt-langplate"
              aria-hidden="true"
              style={{
                left: `${String(table.languagePlate.left)}px`,
                top: `${String(table.languagePlate.top)}px`,
                height: `${String(table.languagePlate.height)}px`,
                padding: table.languagePlate.padding,
              }}
            >
              {bands.languageCode}
            </span>
          )}
          <ConditionDotMark
            signal={bands.conditionSignal}
            isReference={props.isReference}
            isArchived={props.isArchived}
            surface={props.surface}
          />
          {bands.pin === null ? null : <PinControl {...bands.pin} />}

          {/* Band 2 — the hairline, whose stops differ per surface. */}
          <span
            className="cdt-hairline"
            aria-hidden="true"
            style={{
              top: `${String(table.bands.hairlineTop)}px`,
              backgroundImage:
                `linear-gradient(90deg, transparent, rgb(0 0 0 / .4) ${table.hairlineStops[0]}, ` +
                `rgb(0 0 0 / .4) ${table.hairlineStops[1]}, transparent)`,
            }}
          />

          {/* Band 3 — the rank, which in phase 1 is always the uncomputed treatment. */}
          {bands.rank === null ? null : (
            <div
              className="cdt-rank"
              style={{
                left: table.rank.left,
                width: table.rank.width,
                padding: table.rank.padding,
                top: `${String(table.bands.band3Top)}px`,
                bottom: `calc(100% - ${table.bands.band3Bottom})`,
              }}
            >
              <span className="cdt-rank-glyph" aria-hidden="true">
                {bands.rank.glyph}
              </span>
              <span
                className="cdt-rank-label"
                aria-hidden="true"
                style={{ marginTop: `${String(table.rank.labelMarginTop)}px` }}
              >
                {bands.rank.label}
              </span>
              <span className="cdt-visually-hidden">{bands.rank.accessibleName}</span>
            </div>
          )}

          {/* Band 4 — chips, vent bank, jewel stripe, designation. */}
          {step.showStrip ? (
            <>
              <div
                className="cdt-chips"
                style={{
                  left: `${String(table.chipColumn.left)}px`,
                  right: `${String(table.chipColumn.right)}px`,
                  gap: `${String(table.chipColumn.gap)}px`,
                  top: table.bands.band4Top,
                }}
              >
                {bands.chips.map((chip) => (
                  <span key={chip.id}>
                    <span
                      className="cdt-chip"
                      aria-hidden="true"
                      style={{
                        backgroundColor: token(chip.fillToken),
                        color: token(chip.inkToken),
                      }}
                    >
                      {chip.text}
                    </span>
                    <span className="cdt-visually-hidden">{chip.accessibleName}</span>
                  </span>
                ))}
                <span
                  className="cdt-designation"
                  aria-hidden="true"
                  style={{ marginTop: `${String(table.designation.marginTop)}px` }}
                >
                  {bands.designation}
                </span>
              </div>
              <span
                className="cdt-vent"
                aria-hidden="true"
                style={{
                  left: `${String(table.vent.left)}px`,
                  right: `${String(table.vent.right)}px`,
                  bottom: table.vent.bottom,
                  height: `${String(table.vent.height)}px`,
                }}
              />
              <span
                className="cdt-stripe"
                aria-hidden="true"
                style={{
                  bottom: table.jewelStripe.bottom,
                  height: `${String(table.jewelStripe.height)}px`,
                  boxShadow: table.jewelStripe.shadow,
                }}
              />
            </>
          ) : null}

          {/* Band 5 — the scrim container. Its content belongs to the caller. */}
          <div
            className="cdt-scrim"
            style={{
              padding: table.scrim.padding,
              backgroundImage: `linear-gradient(transparent, rgb(7 9 11 / .93) ${table.scrim.stop})`,
            }}
          >
            {props.children}
          </div>
        </CardPlate>
      </div>
    </div>
  );
}

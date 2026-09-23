/**
 * §11.3a's chrome, with §8.7's snaps applied. Colours are `var(--token, <token value>)` so a
 * module renders correctly before the stylesheet mounts and resolves to the token after, and
 * `styles.test.ts` reads every fallback back out of `theme/tokens.ts` so the two cannot drift.
 *
 * This file is the only place any of these numbers is written. `check-style-tokens.mjs` scans
 * `.css` and never sees a style object, so criterion 46's three rules are applied to this
 * module in its own test instead.
 */
import type { CSSProperties } from 'react';
import { REDUCED_CLAMP_MS, allowsTransforms, type ResolvedTier } from '../motion/tier.js';
import { token, tokenValue, type TokenName } from '../theme/tokens.js';

export const DRAWER_WIDTH_PX = 400;

/** §11.3a states both verbatim; §8.7's ladder holds no alpha ground for either. */
export const BACKDROP_SCRIM = 'rgb(6 9 11 / .62)';
export const PANEL_SHADOW = '-20px 0 50px -20px rgb(0 0 0 / .7)';

/**
 * R35(b): `viewIn` is shared and is declared in `styles/base.css` alone. The drawer consumes
 * the name; a screen-local copy would be invisible to `motion.css`'s tier clamp.
 */
export const BACKDROP_KEYFRAME = 'viewIn';

/** This surface's own rail slide, declared once in `styles/motion.css`. */
export const PANEL_KEYFRAME = 'sd-slide-rail';

export const PANEL_CURVE = 'cubic-bezier(.2,.85,.2,1)';
export const BACKDROP_MS = 200;
export const PANEL_MS = 300;

/** `var(--name, <the value that token holds>)`. One owner per value, checked in the test. */
const v = (name: TokenName): string => `${token(name).slice(0, -1)}, ${tokenValue(name)})`;

const surface1 = v('surface-1');
const surface3 = v('surface-3');
const line1 = v('line-1');
const line2 = v('line-2');
const line4 = v('line-4');
const line5 = v('line-5');
const text1 = v('text-1');
const text2 = v('text-2');
const text3 = v('text-3');
const sig = v('sig');
const sigInk = v('sig-ink');
const display = v('font-display');
const body = v('font-body');
const mono = v('font-mono');

export type SettingsStyleKey =
  | 'backdrop'
  | 'panel'
  | 'header'
  | 'title'
  | 'escChip'
  | 'body'
  | 'group'
  | 'groupHeader'
  | 'groupTitle'
  | 'groupRule'
  | 'groupCaption'
  | 'privacyCaption'
  | 'rows'
  | 'row'
  | 'rowText'
  | 'rowLabel'
  | 'rowNote'
  | 'rowValue'
  | 'rowActions'
  | 'switchTrack'
  | 'switchTrackOn'
  | 'switchKnob'
  | 'switchKnobOn'
  | 'statementTrack'
  | 'buttonFilled'
  | 'buttonOutline'
  | 'buttonSmall'
  | 'buttonDashed'
  | 'chip'
  | 'monoValue'
  | 'blockAbsent'
  | 'footer';

export const SD: Readonly<Record<SettingsStyleKey, CSSProperties>> = {
  backdrop: {
    position: 'fixed',
    inset: 0,
    display: 'flex',
    justifyContent: 'flex-end',
    background: BACKDROP_SCRIM,
  },
  panel: {
    width: `${DRAWER_WIDTH_PX}px`,
    maxWidth: '100%',
    height: '100%',
    overflowY: 'auto',
    background: surface1,
    borderLeft: `1px solid ${line4}`,
    boxShadow: PANEL_SHADOW,
  },
  header: {
    display: 'flex',
    alignItems: 'center',
    gap: '10px',
    padding: '13px 16px',
    background: surface3,
    borderBottom: `1px solid ${line2}`,
  },
  title: {
    fontFamily: display,
    fontSize: '15px',
    fontWeight: 700,
    letterSpacing: '.18em',
    color: text1,
    whiteSpace: 'nowrap',
  },
  escChip: {
    display: 'inline-flex',
    alignItems: 'center',
    height: '24px',
    padding: '0 10px',
    marginLeft: 'auto',
    border: `1px solid ${line4}`,
    fontFamily: mono,
    fontSize: '9px',
    letterSpacing: '.14em',
    color: text3,
    whiteSpace: 'nowrap',
  },
  body: {
    display: 'flex',
    flexDirection: 'column',
    padding: '16px 16px 34px',
    gap: '24px',
  },

  group: { display: 'flex', flexDirection: 'column' },
  groupHeader: {
    display: 'flex',
    alignItems: 'baseline',
    gap: '9px',
    marginBottom: '4px',
  },
  groupTitle: {
    fontFamily: display,
    fontSize: '13px',
    fontWeight: 700,
    letterSpacing: '.18em',
    color: text2,
    whiteSpace: 'nowrap',
  },
  groupRule: { flex: 1, minWidth: '6px', height: '1px', background: line2 },
  groupCaption: {
    fontFamily: mono,
    fontSize: '8px',
    letterSpacing: '.12em',
    color: text3,
    whiteSpace: 'nowrap',
  },
  // §8.7 gives the privacy caption its own row — 11.5px at `--text-2` — because it is the
  // consent boundary and is read to decide. It is a sentence under the chips, not a nowrap
  // qualifier in the group header, so it is not the caption slot's style.
  privacyCaption: {
    fontFamily: mono,
    fontSize: '11.5px',
    letterSpacing: '.1em',
    lineHeight: 1.6,
    color: text2,
    textWrap: 'pretty',
    paddingTop: '9px',
  },

  rows: { display: 'flex', flexDirection: 'column' },
  row: {
    display: 'flex',
    alignItems: 'flex-start',
    gap: '11px',
    padding: '10px 0',
    borderBottom: `1px solid ${line1}`,
  },
  rowText: { display: 'flex', flexDirection: 'column', gap: '4px', flex: 1, minWidth: 0 },
  rowLabel: { fontFamily: body, fontSize: '12.5px', lineHeight: 1.35, color: text1 },
  rowNote: {
    fontFamily: mono,
    fontSize: '8px',
    letterSpacing: '.1em',
    color: text3,
    textWrap: 'pretty',
  },
  rowValue: {
    fontFamily: mono,
    fontSize: '10.5px',
    letterSpacing: '.1em',
    color: text2,
    overflowWrap: 'anywhere',
  },
  rowActions: { display: 'flex', alignItems: 'center', gap: '9px', flex: 'none' },

  // 26 × 14, a square track with an inset edge. The knob travels 12px, which is the track
  // minus the knob and its two margins — one number, not a second position to keep in step.
  switchTrack: {
    flex: 'none',
    width: '26px',
    height: '14px',
    marginTop: '2px',
    padding: 0,
    border: 'none',
    background: line1,
    boxShadow: `inset 0 0 0 1px ${line4}`,
    cursor: 'pointer',
  },
  switchTrackOn: { background: sig, boxShadow: `inset 0 0 0 1px ${sig}` },
  // A block, or the `<span>` stays inline inside its `<button>` and its box is ignored: the knob
  // measured 0px wide in the built app and every switch read as a bare coloured bar.
  switchKnob: {
    display: 'block',
    width: '10px',
    height: '10px',
    margin: '2px',
    background: text3,
  },
  switchKnobOn: { background: sigInk, transform: 'translateX(12px)' },
  // §11.3a's statement variant, verbatim: the track collapses to 3px at top 7px, the edge goes
  // transparent, the knob is gone and the cursor is default. It is not a disabled switch —
  // `rows.tsx` gives it no interactive role either.
  statementTrack: {
    flex: 'none',
    width: '26px',
    height: '3px',
    marginTop: '7px',
    background: sig,
    boxShadow: 'inset 0 0 0 1px transparent',
    cursor: 'default',
  },

  buttonFilled: {
    display: 'inline-flex',
    alignItems: 'center',
    height: '28px',
    padding: '0 13px',
    border: 'none',
    background: sig,
    color: sigInk,
    fontFamily: display,
    fontSize: '12px',
    fontWeight: 700,
    letterSpacing: '.13em',
    whiteSpace: 'nowrap',
    cursor: 'pointer',
  },
  buttonOutline: {
    display: 'inline-flex',
    alignItems: 'center',
    height: '28px',
    padding: '0 13px',
    border: `1px solid ${line5}`,
    background: 'transparent',
    color: text1,
    fontFamily: display,
    fontSize: '12px',
    fontWeight: 600,
    letterSpacing: '.12em',
    whiteSpace: 'nowrap',
    cursor: 'pointer',
  },
  buttonSmall: {
    display: 'inline-flex',
    alignItems: 'center',
    height: '23px',
    padding: '0 9px',
    border: `1px solid ${line4}`,
    background: 'transparent',
    color: text3,
    fontFamily: mono,
    fontSize: '8px',
    letterSpacing: '.12em',
    whiteSpace: 'nowrap',
    cursor: 'pointer',
  },
  buttonDashed: {
    display: 'inline-flex',
    alignItems: 'center',
    padding: '3px 7px',
    border: `1px dashed ${line5}`,
    background: 'transparent',
    color: sig,
    fontFamily: mono,
    fontSize: '8px',
    letterSpacing: '.12em',
    whiteSpace: 'nowrap',
    cursor: 'pointer',
  },
  chip: {
    padding: '2px 6px',
    border: `1px solid ${line2}`,
    fontFamily: mono,
    fontSize: '8px',
    letterSpacing: '.12em',
    color: text3,
    whiteSpace: 'nowrap',
  },
  monoValue: {
    fontFamily: mono,
    fontSize: '9px',
    letterSpacing: '.1em',
    color: text3,
    whiteSpace: 'nowrap',
  },

  // §11.3a: amber says *something is wrong here*; in phase 1 an absent token is the shipped
  // state, so the GitHub and notification blocks take `--absent`, never the accent. `--line-4`
  // and `--absent` hold the same value and this is a line, so the line token is the one used.
  blockAbsent: {
    borderLeft: `2px solid ${line4}`,
    paddingLeft: '11px',
    display: 'flex',
    flexDirection: 'column',
    gap: '9px',
  },
  footer: {
    display: 'flex',
    flexDirection: 'column',
    gap: '4px',
    paddingTop: '11px',
    fontFamily: mono,
    fontSize: '8px',
    letterSpacing: '.14em',
    lineHeight: 1.8,
    color: text3,
  },
};

/** The scrim is opacity only, so it survives `reduced` clamped rather than being dropped. */
export function backdropAnimation(tier: ResolvedTier): CSSProperties {
  if (tier === 'off') return { animation: 'none' };
  const ms = allowsTransforms(tier) ? BACKDROP_MS : REDUCED_CLAMP_MS;
  return { animation: `${BACKDROP_KEYFRAME} ${String(ms)}ms both` };
}

/** §11.6: `reduced` drops the transform, so the panel fades in where it will stand. */
export function panelAnimation(tier: ResolvedTier): CSSProperties {
  if (tier === 'off') return { animation: 'none' };
  if (!allowsTransforms(tier)) {
    return { animation: `${BACKDROP_KEYFRAME} ${String(REDUCED_CLAMP_MS)}ms both` };
  }
  return { animation: `${PANEL_KEYFRAME} ${String(PANEL_MS)}ms ${PANEL_CURVE} both` };
}

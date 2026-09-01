/**
 * §11.2a's full-screen idiom, shared by the four startup windows. §10.1a's roots screen supplies
 * the frame; this module supplies the failure treatment: `--fail-hot` on the eyebrow, and the
 * filled `--sig` bar reserved for the safe action.
 *
 * `--sig` is refused on the eyebrow because it means *system, alive*, and a window that exists
 * because the system is not alive may not wear it. The same rule puts `FORCE` in the outline
 * button: the filled bar is the safe action everywhere else in this product, so the forcing
 * action is never in it.
 *
 * `@keyframes viewIn` is declared in `renderer/styles/base.css` and is used here by name only —
 * a second declaration would silently override it and would sit outside `motion.css`'s tier
 * clamp. It is opacity-only, which is why `reduced` may keep it at all; the tier is applied
 * here rather than by the clamp because these windows can be on screen precisely because the
 * GPU is what broke and `paint_fail_count` has forced `off`.
 *
 * The seconds are **read, not counted**. A counter incremented per tick lies whenever a timer
 * is throttled or coalesced, and a window titled *still shutting down* is exactly where the
 * machine is busy enough for that to happen.
 */
import { useEffect, useRef, useState } from 'react';
import type { CSSProperties, ReactElement } from 'react';

import { REDUCED_CLAMP_MS, type ResolvedTier } from '../motion/tier';
import {
  corruptLedger,
  failureCopy,
  forceIsOffered,
  logPathNote,
  shuttingDownNote,
  type FailureFact,
} from './copy';

export const FAILURE_ENTER_MS = 300;

export interface FailureWindowProps {
  readonly fact: FailureFact;
  readonly logPath: string;
  readonly tier: ResolvedTier;
  readonly nowMs: () => number;
  readonly onPrimary: () => void;
  readonly onSecondary: () => void;
}

const rootStyle: CSSProperties = {
  position: 'absolute',
  inset: 0,
  zIndex: 50,
  overflowY: 'auto',
  background: 'var(--surface-0, #0a0d10)',
};
const columnStyle: CSSProperties = {
  maxWidth: 760,
  margin: '0 auto',
  padding: '52px 30px 40px',
  display: 'flex',
  flexDirection: 'column',
  gap: 18,
};
const eyebrowStyle: CSSProperties = {
  fontFamily: 'var(--font-mono, ui-monospace)',
  fontSize: 8.5,
  lineHeight: 1,
  letterSpacing: '.32em',
  color: 'var(--fail-hot, #e0533d)',
};
const headlineStyle: CSSProperties = {
  margin: 0,
  fontFamily: 'var(--font-display, system-ui)',
  fontSize: 42,
  fontWeight: 700,
  lineHeight: 1.06,
  color: 'var(--text-0, #f2f5f7)',
};
const bodyStyle: CSSProperties = {
  margin: 0,
  maxWidth: 560,
  fontFamily: 'var(--font-body, system-ui)',
  fontSize: 13.5,
  lineHeight: 1.6,
  color: 'var(--text-3, #8b97a3)',
  textWrap: 'pretty',
};
const noteStyle: CSSProperties = {
  fontFamily: 'var(--font-mono, ui-monospace)',
  fontSize: 9,
  lineHeight: 1,
  letterSpacing: '.12em',
  color: 'var(--text-3, #8b97a3)',
};
const blockLabelStyle: CSSProperties = {
  fontFamily: 'var(--font-mono, ui-monospace)',
  fontSize: 8,
  lineHeight: 1,
  letterSpacing: '.2em',
  color: 'var(--text-3, #8b97a3)',
};
const primaryStyle: CSSProperties = {
  height: 42,
  padding: '0 30px',
  background: 'var(--sig, #4a9dff)',
  border: 'none',
  color: 'var(--sig-ink, #08131f)',
  fontFamily: 'var(--font-display, system-ui)',
  fontSize: 15,
  fontWeight: 700,
  lineHeight: 1,
  letterSpacing: '.18em',
  cursor: 'pointer',
};
const secondaryStyle: CSSProperties = {
  height: 42,
  padding: '0 18px',
  background: 'transparent',
  border: '1px solid var(--line-5, #3c454e)',
  color: 'var(--text-2, #b6c1cb)',
  fontFamily: 'var(--font-display, system-ui)',
  fontSize: 13,
  fontWeight: 600,
  lineHeight: 1,
  letterSpacing: '.12em',
  cursor: 'pointer',
};

function enterAnimation(tier: ResolvedTier): CSSProperties {
  if (tier === 'off') return {};
  const ms = tier === 'reduced' ? REDUCED_CLAMP_MS : FAILURE_ENTER_MS;
  return { animation: `viewIn ${String(ms)}ms both` };
}

/** Re-reads the clock on a one-second tick. The value is a difference, never an accumulator. */
function useWallClock(active: boolean, nowMs: () => number): number {
  const [value, setValue] = useState(() => nowMs());
  useEffect(() => {
    if (!active) return undefined;
    const id = setInterval(() => {
      setValue(nowMs());
    }, 1000);
    return () => {
      clearInterval(id);
    };
  }, [active, nowMs]);
  return value;
}

export function FailureWindow(props: FailureWindowProps): ReactElement {
  const { fact, logPath, tier, nowMs, onPrimary, onSecondary } = props;
  const copy = failureCopy(fact);
  const waiting = fact.kind === 'still_shutting_down';
  const now = useWallClock(waiting, nowMs);
  const primaryRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    primaryRef.current?.focus();
  }, []);

  const startedAtMs = fact.kind === 'still_shutting_down' ? fact.startedAtMs : 0;
  const showSecondary = copy.secondary !== null && (!waiting || forceIsOffered(startedAtMs, now));
  const note = waiting ? shuttingDownNote(startedAtMs, now) : null;

  return (
    <div data-testid="fw-root" style={{ ...rootStyle, ...enterAnimation(tier) }}>
      <div style={columnStyle}>
        <span data-testid="fw-eyebrow" style={eyebrowStyle}>
          {copy.eyebrow}
        </span>
        <h1 data-testid="fw-headline" style={headlineStyle}>
          {copy.headline}
        </h1>
        {copy.body.map((line) => (
          <p key={line.slice(0, 24)} style={bodyStyle}>
            {line}
          </p>
        ))}
        {fact.kind === 'corrupt_index'
          ? corruptLedger(fact).map((block, index) => (
              <div
                key={block.label}
                data-testid={`fw-block-${String(index)}`}
                style={{ display: 'flex', flexDirection: 'column', gap: 5 }}
              >
                <span data-testid="fw-block-label" style={blockLabelStyle}>
                  {block.label}
                </span>
                {block.lines.map((line) => (
                  <span key={line} style={bodyStyle}>
                    {line}
                  </span>
                ))}
              </div>
            ))
          : null}
        {note !== null ? (
          <span data-testid="fw-note" style={noteStyle} role="status">
            {note}
          </span>
        ) : null}
        <span data-testid="fw-log" style={noteStyle}>
          {logPathNote(logPath)}
        </span>
        <div style={{ display: 'flex', gap: 10 }}>
          <button ref={primaryRef} type="button" style={primaryStyle} onClick={onPrimary}>
            {copy.primary}
          </button>
          {showSecondary ? (
            <button type="button" style={secondaryStyle} onClick={onSecondary}>
              {copy.secondary}
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}

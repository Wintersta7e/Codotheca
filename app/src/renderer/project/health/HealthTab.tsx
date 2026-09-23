/**
 * [p3] §30.7 — the `HEALTH` tab: **the reading**, the pair of quantities in two units that are
 * never combined, the basis, and every unknown named.
 *
 * **No surface combines the pair into one scalar.** `scoredOpen` counts items and the basis
 * counts checks; each carries its unit in its name and neither is derived from the other. A
 * single figure over the two is the thing §30.1 exists to prevent.
 *
 * **§30 touches no motion**; the restoration surge is §34's. Decision-carrying text sits at
 * `--text-3` or lighter.
 */
import type { ReactElement } from 'react';

import type { HealthReading, Settings } from '../../../generated/protocol';
import { WORKTREE_STALE_AFTER_SECS } from '../../derive/observation';
import {
  basisLine,
  formFor,
  grantMissingCount,
  needsSourceGrant,
  offCause,
  openLine,
  PART_SEPARATOR,
} from './checkForms';
import { GrantAsk } from './GrantAsk';

export interface HealthTabProps {
  readonly reading: HealthReading;
  readonly settings: Settings;
  readonly now: number;
  /** Applies `SettingsPatch.contentScanEnabled = true` through the existing `settings.set`. */
  readonly onGrantSourceReading: () => void;
}

/**
 * §6's staleness threshold, **imported by name and never restated** (R125). The constant lives at
 * exactly one site, `app/src/renderer/derive/observation.ts`, whose own comment says *"§6 … names
 * no number. This is that number."* A second literal here is the defect that rule was written
 * against.
 *
 * A `frozen` reading always renders its age; a `live` one renders its age past this threshold.
 */
function ageLine(observedAt: number, now: number, frozen: boolean): string | null {
  const age = now - observedAt;
  if (!frozen && age < WORKTREE_STALE_AFTER_SECS) return null;
  const minutes = Math.max(1, Math.round(age / 60));
  const suffix = frozen ? ' · under glass while the store is away' : '';
  return `as of ${minutes} min ago${suffix}`;
}

export function HealthTab({
  reading,
  settings,
  now,
  onGrantSourceReading,
}: HealthTabProps): ReactElement {
  const basis = reading.basis;
  const coverage = basisLine(basis, reading.state, grantMissingCount(reading.checks, settings));
  const open = openLine(reading.scoredOpen, basis);
  const age = basis === null ? null : ageLine(basis.observedAt, now, reading.state === 'frozen');

  return (
    <section className="cp-health" data-testid="cp-health" aria-label="Project health">
      <header className="cp-health-head">
        {open === null ? null : (
          <p className="cp-health-open" data-testid="cp-health-open">
            {open}
          </p>
        )}
        {coverage === null ? null : (
          <p className="cp-health-basis" data-testid="cp-health-basis">
            {coverage}
          </p>
        )}
        {age === null ? null : (
          <p className="cp-health-age" data-testid="cp-health-age">
            {age}
          </p>
        )}
      </header>
      <ul className="cp-health-checks">
        {reading.checks.map((check) => {
          const cause = check.outcome === 'off' ? offCause(check, settings) : null;
          const form = formFor(check, cause);
          return (
            // The parts are separated in the text itself, as the debt list's are, so the row
            // reads as parts before any stylesheet does: never `missing_readmeOPEN`.
            <li key={check.id} className="cp-health-check" data-outcome={check.outcome}>
              <span className="cp-health-check-id">{check.id}</span>
              {PART_SEPARATOR}
              <span className="cp-health-check-word">{form.word}</span>
              {form.detail === '' ? null : (
                <>
                  {PART_SEPARATOR}
                  <span className="cp-health-check-detail">{form.detail}</span>
                </>
              )}
              {cause === 'switchedOff' ? (
                <>
                  {PART_SEPARATOR}
                  <span className="cp-health-check-detail">you switched this check off</span>
                </>
              ) : null}
              {cause === 'grantMissing' && needsSourceGrant(check.id) ? (
                <GrantAsk onGrant={onGrantSourceReading} />
              ) : null}
            </li>
          );
        })}
      </ul>
    </section>
  );
}

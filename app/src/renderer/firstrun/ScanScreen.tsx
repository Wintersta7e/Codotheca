import type { ReactElement } from 'react';
import { FOUND_LABEL, SO_FAR_QUALIFIER } from './copy';
import { SCAN_COUNT_ROLE, SKIP_AHEAD_LABEL } from '../a11y/names';
import { scanLineText } from './scanFeed';
import type { ScanFeedState, ScanTile } from './scanFeed';
import type { RevealPanel } from './revealModel';
import type { ResolvedTier } from '../motion/tier';

export interface ScanScreenDeps {
  /** `null` is *not yet classified*: the stripe stays and paints `--absent`. */
  readonly jewelFor: (tile: ScanTile) => string | null;
  readonly onSkipAhead: () => void;
  readonly onOpenScanSummary: () => void;
}

export interface ScanScreenProps {
  readonly deps: ScanScreenDeps;
  readonly feed: ScanFeedState;
  /** `<root> · <root> · 3 ROOTS`, assembled by the gate. */
  readonly rootLine: string;
  /** §10.3a's preview, already computed from the same `stats.reveal` as the reveal itself. */
  readonly milestone: RevealPanel | null;
  readonly tier: ResolvedTier;
}

/**
 * §11.7: the count is the only progress signal, and a number that is merely repainted is never
 * announced. The reading changes only at a milestone and at completion, so it is paced rather
 * than flooded across a pass that measures 75–120 s at 1,000 repositories.
 */
export function announcement(feed: ScanFeedState, milestone: RevealPanel | null): string | null {
  if (feed.finished) return `${String(feed.found)} found`;
  if (milestone !== null && milestone.value !== null) {
    return `${String(feed.found)} found · ${milestone.label} ${milestone.value} ${SO_FAR_QUALIFIER}`;
  }
  return null;
}

export function ScanScreen(props: ScanScreenProps): ReactElement {
  const { deps, feed, rootLine, milestone, tier } = props;
  const reading = announcement(feed, milestone);

  return (
    <div className="cdt-fr-view cdt-fr-view--scan" data-effects-tier={tier}>
      <div className="cdt-fr-beam-well" aria-hidden="true">
        <div className="cdt-fr-beam" data-testid="fr-beam" aria-hidden="true" />
      </div>

      <div className="cdt-fr-scan-head">
        <div className="cdt-fr-rootline">
          <span className="cdt-fr-rootline-dot" aria-hidden="true" />
          <span className="cdt-fr-rootline-text">{rootLine}</span>
        </div>

        <div className="cdt-fr-found-line">
          {/* Hidden from the reading: the role="status" line below is what is announced. */}
          <span className="cdt-fr-count-big" data-testid="fr-count-big" aria-hidden="true">
            {feed.found}
          </span>
          <span className="cdt-fr-found" aria-hidden="true">
            {FOUND_LABEL}
          </span>
        </div>
        <span role={SCAN_COUNT_ROLE} className="cdt-fr-sr">
          {reading ?? ''}
        </span>

        {/* §11.7 and §10.3a: a real button, in the first rendered frame, unconditionally. */}
        <button type="button" className="cdt-fr-skip" onClick={deps.onSkipAhead}>
          {SKIP_AHEAD_LABEL}
        </button>

        <div className="cdt-fr-tally">
          {feed.tally.map((chip) => (
            <span className="cdt-fr-tally-chip" key={chip.sigil}>
              <span className="cdt-fr-tally-sigil">{chip.sigil}</span>
              <span className="cdt-fr-tally-count">{chip.count}</span>
            </span>
          ))}
        </div>

        {/* Skipped silently when the figure is not yet computable: no zero, no dash. */}
        {milestone !== null && milestone.value !== null ? (
          <div className="cdt-fr-milestone" data-testid="fr-milestone">
            <span className="cdt-fr-milestone-key">{milestone.label}</span>
            <span className="cdt-fr-milestone-value">{milestone.value}</span>
            <span className="cdt-fr-milestone-sofar">{SO_FAR_QUALIFIER}</span>
          </div>
        ) : null}
      </div>

      <div className="cdt-fr-grid-well">
        <div className="cdt-fr-grid">
          {feed.tiles.map((tile) => {
            const jewel = deps.jewelFor(tile);
            return (
              <div
                className="cdt-fr-tile"
                data-testid="fr-tile"
                data-merged={feed.merging.has(tile.id)}
                key={tile.id}
              >
                <div className="cdt-fr-tile-plate">
                  <div
                    className="cdt-fr-tile-stripe"
                    data-testid="fr-tile-stripe"
                    data-lit={jewel !== null}
                    // A missing stripe reads *no language*; the absent-grey default reads
                    // *language not yet known*, so the inline colour is only ever an override.
                    style={jewel === null ? undefined : { background: jewel }}
                  />
                  <div className="cdt-fr-tile-scrim">
                    <div className="cdt-fr-tile-name">{tile.name}</div>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      <button type="button" className="cdt-fr-scanline" onClick={deps.onOpenScanSummary}>
        {scanLineText(feed.walkedDirs, feed.foundRepos)}
      </button>
    </div>
  );
}

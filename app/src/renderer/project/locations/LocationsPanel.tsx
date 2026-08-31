/**
 * §8.5.2. §2.4 ships `locations.relocate` and a `projects.launch` carrying a `locationId`, and
 * §1.3 stores a four-state presence; no phase-1 surface reached any of the three. This panel is
 * where they become reachable, and it leads the Overview tab because its only job is answering
 * which copy is current.
 *
 * §17: nothing here mutates disk. `RELOCATE` rewrites one `location` row's path and is the only
 * write; `ENABLE ROOT` writes a root's enabled flag by id, with no path on the wire either way.
 */
import { useRef, type CSSProperties, type ReactElement } from 'react';

import type {
  LocationDetail,
  LocationId,
  ProjectDetail,
  TargetRow,
} from '../../../generated/protocol';
import { appearanceFor, fadeFor, jewelAlpha, seedOf } from '../../art/appearance';
import { useProjectPageDeps } from '../deps';
import { cascadeDelay } from '../motion';
import {
  actionLabel,
  footerText,
  headerNote,
  kindChip,
  locationActions,
  locationFacts,
  locationNote,
  rowTag,
  stateWord,
  type LocationActionId,
} from './locationCopy';

export function fileManagerTarget(targets: readonly TargetRow[]): TargetRow | null {
  const found = [...targets.filter((t) => t.kind === 'file_manager')].sort(
    (a, b) => a.sortIndex - b.sortIndex,
  );
  return found[0] ?? null;
}

export interface LocationsPanelProps {
  detail: ProjectDetail;
  shownId: LocationId | null;
  onShow: (id: LocationId) => void;
  onChanged: () => void;
}

export function LocationsPanel({
  detail,
  shownId,
  onShow,
  onChanged,
}: LocationsPanelProps): ReactElement | null {
  const deps = useProjectPageDeps();
  // §2.4: launch is non-idempotent, so the guard is a ref and holds within the tick as well as
  // across the re-render a state flag would have to wait for.
  const busy = useRef(false);
  if (detail.locations.length === 0) return null;

  const rows = [...detail.locations].sort((a, b) => Number(b.isPrimary) - Number(a.isPrimary));
  const primaryId = rows.find((r) => r.isPrimary)?.location.id ?? rows[0]?.location.id ?? null;
  const shown = shownId ?? primaryId;
  const revealTarget = fileManagerTarget(detail.targets);

  // §7.3a's derivation, through the one function the card already uses: the primary row is this
  // project's colour here exactly as it is on its tile, from one owner rather than two.
  const appearance = appearanceFor(
    seedOf(detail.row),
    fadeFor(detail.row),
    detail.row.primaryLanguage,
  );

  const launch = (location: LocationDetail, targetId: TargetRow['id'] | null): void => {
    if (busy.current) return;
    busy.current = true;
    void (async () => {
      try {
        let id = targetId;
        if (id === null) {
          // §4bis.5: this is the call that makes a per-location override reachable at all.
          const list = await deps.request('targets.list', {
            projectId: detail.row.id,
            locationId: location.location.id,
          });
          id = list.resolved?.target.id ?? detail.resolvedTarget?.target.id ?? null;
        }
        if (id === null) return;
        await deps.request('projects.launch', {
          projectId: detail.row.id,
          locationId: location.location.id,
          targetId: id,
        });
      } catch {
        // §11.4's failure window owns the copy; the panel states nothing it cannot know.
      } finally {
        busy.current = false;
      }
    })();
  };

  const run = (location: LocationDetail, action: LocationActionId): void => {
    if (action === 'open') {
      launch(location, null);
      return;
    }
    if (action === 'reveal') {
      launch(location, revealTarget?.id ?? null);
      return;
    }
    if (action === 'relocate') {
      void deps.relocate(location.location.id).then((reply) => {
        if (reply.kind === 'relocated') onChanged();
      });
      return;
    }
    if (location.coveringRootId !== null) {
      void deps
        .request('roots.setEnabled', { id: location.coveringRootId, enabled: true })
        .then(onChanged)
        .catch(() => undefined);
    }
  };

  const footer = footerText(detail.associationKind, rows.length);

  return (
    <section
      className="cp-loc cp-rise"
      style={
        {
          animationDelay: cascadeDelay(1),
          '--cdt-jewel': appearance.jewel,
          '--cdt-jewel-55': jewelAlpha(appearance, 0.55),
        } as CSSProperties
      }
    >
      <div className="cp-loc-header" data-testid="cp-loc-header">
        {headerNote(rows)}
      </div>
      <ul className="cp-loc-list">
        {rows.map((row) => {
          const word = stateWord(row);
          const note = locationNote(row);
          const warn = row.presence === 'present' && row.headComparison === 'different_commit';
          const facts = locationFacts({
            location: row,
            sizeTrackedBytes: detail.row.sizeTrackedBytes,
            now: deps.now(),
          });
          const actions = locationActions(row).filter(
            (a) => a !== 'reveal' || revealTarget !== null,
          );
          const cls = [
            'cp-loc-row',
            row.isPrimary ? 'cp-loc-row-primary' : '',
            warn ? 'cp-loc-row-warn' : '',
          ]
            .filter(Boolean)
            .join(' ');
          return (
            <li key={String(row.location.id)}>
              <div className={cls} data-testid="cp-loc-row">
                <button
                  type="button"
                  className="cp-loc-adopt"
                  data-testid="cp-loc-adopt"
                  aria-pressed={row.location.id === shown}
                  disabled={row.presence !== 'present'}
                  onClick={() => {
                    onShow(row.location.id);
                  }}
                >
                  <span className="cp-loc-chip">{kindChip(row.kind, row.distro)}</span>
                  <span className="cp-loc-path" title={row.location.pathDisplay}>
                    {row.location.pathDisplay}
                  </span>
                  <span className="cp-loc-facts">
                    {facts.map((f) => (
                      <span key={f.key}>
                        <span className="cp-loc-fact-key">{f.key} </span>
                        <span className="cp-loc-fact-value">{f.value}</span>
                      </span>
                    ))}
                  </span>
                </button>
                <span className="cp-loc-state" data-testid="cp-loc-state">
                  {word}
                </span>
                <span
                  className={row.isPrimary ? 'cp-loc-tag cp-loc-tag-primary' : 'cp-loc-tag'}
                  data-testid="cp-loc-tag"
                >
                  {rowTag(row.isPrimary)}
                </span>
                {actions.map((a) => (
                  <button
                    key={a}
                    type="button"
                    className="cp-loc-action"
                    onClick={() => {
                      run(row, a);
                    }}
                  >
                    {actionLabel(a)}
                  </button>
                ))}
              </div>
              {note === null ? null : (
                <p
                  className={warn ? 'cp-loc-note cp-loc-note-warn' : 'cp-loc-note'}
                  data-testid="cp-loc-note"
                >
                  {note}
                </p>
              )}
            </li>
          );
        })}
      </ul>
      {footer === null ? null : (
        <div className="cp-loc-footer" data-testid="cp-loc-footer">
          {footer}
        </div>
      )}
    </section>
  );
}

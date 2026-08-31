/**
 * §8.5's `OPENS IN`, which is the per-project override — the narrowest scope in the target
 * resolution order. `targets.setDefault` carries the whole scope triple, and all three NULL is
 * the global default, so a project override names `projectId` and NULLs the other two.
 *
 * The tier is not derivable in the renderer, which is why the resolved row arrives carrying the
 * tier that resolved it: the menu labels a project-scoped row `SET` and every other tier
 * `DEFAULT`. The detail payload already carries it, so opening the menu issues no command.
 */
import { forwardRef, useState, type ReactElement } from 'react';

import type {
  ProjectId,
  ResolvedTarget,
  TargetRow,
  TargetTier,
  VerifyState,
} from '../../../generated/protocol';
import { useProjectPageDeps } from '../deps';

/** The terminal has its own button on this rail, so it is not offered here. */
export function editorTargets(targets: readonly TargetRow[]): TargetRow[] {
  return [...targets].filter((t) => t.kind === 'editor').sort((a, b) => a.sortIndex - b.sortIndex);
}

export function tierLabel(tier: TargetTier): 'DEFAULT' | 'SET' {
  return tier === 'project' ? 'SET' : 'DEFAULT';
}

/** §11.5: `verify_state` renders on the target in the dropdown. A verified one says nothing. */
export function verifyNote(state: VerifyState): string | null {
  if (state === 'ok') return null;
  return state === 'missing' ? 'NOT FOUND WHERE IT WAS' : 'NOT CHECKED YET';
}

export const OPENS_IN_FOOTER = 'SET FOR THIS PROJECT ONLY · THE DEFAULT LIVES IN SETTINGS';

export interface OpensInProps {
  projectId: ProjectId;
  targets: readonly TargetRow[];
  resolved: ResolvedTarget | null;
  onChanged: () => void;
}

/**
 * The ref is the trigger button, never the wrapper: when nothing resolves, the primary control
 * reads `CHOOSE AN APP` and answers a press by moving focus to the control that chooses the app.
 * Focusing a `<div>` would put the focus ring on nothing, and a control that cannot act is the
 * dead control §11.3a forbids.
 */
export const OpensIn = forwardRef<HTMLButtonElement, OpensInProps>(function OpensIn(
  { projectId, targets, resolved, onChanged }: OpensInProps,
  ref,
): ReactElement | null {
  const deps = useProjectPageDeps();
  const [open, setOpen] = useState(false);
  const editors = editorTargets(targets);
  // Never a dead control: with no editor known there is nothing to choose between.
  if (editors.length === 0) return null;

  const pick = (target: TargetRow): void => {
    setOpen(false);
    deps
      .request('targets.setDefault', {
        targetId: target.id,
        projectId,
        locationId: null,
        language: null,
      })
      .then(onChanged)
      .catch(() => {
        // A write that did not land changes nothing on screen. The rail resyncs from the next
        // detail read rather than asserting an override the core never accepted.
      });
  };

  return (
    <div className="cp-opensin-wrap">
      <button
        ref={ref}
        type="button"
        className="cp-opensin"
        data-testid="cp-opensin"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => {
          setOpen((value) => !value);
        }}
      >
        <span className="cp-opensin-label">OPENS IN</span>
        <span className="cp-opensin-value" data-testid="cp-opensin-value">
          {resolved?.target.name ?? 'CHOOSE'}
        </span>
      </button>
      {open ? (
        <div className="cp-opensin-menu" role="menu">
          {editors.map((target) => {
            const note = verifyNote(target.verifyState);
            const isResolved = resolved !== null && resolved.target.id === target.id;
            const parts = [
              isResolved && resolved !== null ? tierLabel(resolved.tier) : null,
              note,
            ].filter((part): part is string => part !== null);
            return (
              <button
                key={target.id}
                type="button"
                role="menuitem"
                className="cp-opensin-item"
                onClick={() => {
                  pick(target);
                }}
              >
                <span>{target.name}</span>
                {parts.length === 0 ? null : (
                  <span className="cp-opensin-note">{parts.join(' · ')}</span>
                )}
              </button>
            );
          })}
          <div className="cp-opensin-footer" data-testid="cp-opensin-footer">
            {OPENS_IN_FOOTER}
          </div>
        </div>
      ) : null}
    </div>
  );
});

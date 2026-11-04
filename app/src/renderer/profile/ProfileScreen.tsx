/**
 * [p4] §43.13's Profile, minimal: the route host holding §43's two sections and nothing else yet.
 * §42 owns the page and extends this one screen with its own sections and its drawer entry —
 * never a second Profile.
 *
 * Opening it writes nothing: no acknowledgement and no landing (§40.8 — the listing lands
 * nothing). It reads `badges.list` once on mount and opens scrolled to its anchor; Esc returns to
 * the shelf.
 */
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactElement,
  useEffect,
  useRef,
  useState,
} from 'react';

import type { BadgesListed, MotivationState } from '../../generated/protocol';
import type { AppDeps } from '../app/deps';
import type { ProfileAnchor } from '../app/route';
import { PROFILE_BACK_LABEL } from '../motivation/copy';
import { PROFILE_ANCHOR, ProfileBadgesSection, ProfileLevelSection } from './ProfileMotivation';

export interface ProfileScreenProps {
  readonly state: MotivationState | null;
  readonly at: ProfileAnchor;
  readonly request: AppDeps['request'];
  readonly onBack: () => void;
}

/** The anchor's place on the page: the badge a line named when it is listed, else its section. */
function anchorTarget(root: HTMLElement, at: ProfileAnchor): HTMLElement | null {
  const named =
    at.section === 'badges' && at.badge !== undefined
      ? root.querySelector<HTMLElement>(`[${PROFILE_ANCHOR}="badge:${at.badge}"]`)
      : null;
  return named ?? root.querySelector<HTMLElement>(`[${PROFILE_ANCHOR}="${at.section}"]`);
}

export function ProfileScreen(props: ProfileScreenProps): ReactElement {
  const { state, at, request, onBack } = props;
  const [listed, setListed] = useState<BadgesListed | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const shown = state?.shown === true;

  useEffect(() => {
    let mounted = true;
    request('badges.list', {}).then(
      (answer) => {
        if (mounted) setListed(answer);
      },
      // A refused read leaves the badges unread, and nothing is drawn for them.
      () => undefined,
    );
    return () => {
      mounted = false;
    };
  }, [request]);

  // Keyboard focus lands on the page, so Esc reaches it without a click first.
  useEffect(() => {
    rootRef.current?.focus();
  }, []);

  // Scrolled, not animated: the page opens at its anchor, and again once the badges arrive.
  useEffect(() => {
    const root = rootRef.current;
    if (root === null) return;
    const target = anchorTarget(root, at);
    if (target !== null) root.scrollTop = target.offsetTop;
  }, [at, listed, shown]);

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>): void => {
    if (event.key !== 'Escape') return;
    event.preventDefault();
    onBack();
  };

  return (
    <div ref={rootRef} className="cdt-profile" tabIndex={-1} onKeyDown={onKeyDown}>
      <div className="cdt-profile-bar">
        <button type="button" className="cdt-profile-back" onClick={onBack}>
          <span aria-hidden="true">←</span>
          {PROFILE_BACK_LABEL}
        </button>
      </div>
      {state === null ? null : <ProfileLevelSection state={state} />}
      <ProfileBadgesSection listed={listed} shown={shown} />
    </div>
  );
}

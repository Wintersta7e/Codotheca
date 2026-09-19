/**
 * [p3] §32.12 — **the shell posts the one notification phase 3 may fire.**
 *
 * The core observes the transition and emits `sync/advisory_alert`; this posts it. Nothing in
 * `app/src/renderer` may originate an OS notification, the renderer's `notifications` permission
 * stays denied, and `scripts/check-notification-origin.mjs` is what keeps that true.
 *
 * **The sentence is not invented here.** §11.7 forbids a developer's invention on the one surface
 * a user cannot dismiss before reading, so the title is read from the settings drawer's own list —
 * §11.3a's words, already shipped — and the body carries only identifiers the payload named.
 */
import type { AdvisoryAlert, Topic } from '../generated/protocol';

import { ADVISORY_NOTICE_TITLE } from '../shared/notificationCopy';

/**
 * The body, from the payload alone.
 *
 * **No field may express an absence.** Every reason a project was excluded is an absence, and the
 * contract's footer states that no notification mentions one — so there is no excluded count, no
 * suppressed count and no reason, and nothing here invents one either.
 *
 * With several projects it names the count and no project; with one it names the advisory. The
 * CVE id is a **display** field: one advisory carries several or none, and a critical advisory
 * with no CVE id is still notifiable.
 */
export function advisoryNoticeBody(alert: AdvisoryAlert): string {
  if (alert.projectCount !== 1 || alert.advisoryId === null) {
    return `${String(alert.advisoryCount)} advisories across ${String(alert.projectCount)} projects.`;
  }
  const named = alert.cveId ?? alert.advisoryId;
  const pkg = alert.packageName ?? '';
  return pkg === '' ? named : `${pkg} — ${named}`;
}

interface Deps {
  subscribe: (topic: Topic, handler: (event: string, payload: unknown) => void) => () => void;
  notify: (title: string, body: string) => void;
}

/**
 * Subscribe to `sync`, post on `advisory_alert`, and post on **nothing else**.
 *
 * Returns the unsubscribe the caller owns. It subscribes eagerly rather than on demand: the alert
 * is not a status delta and is never replayed, so a subscriber that was not listening has missed
 * it.
 */
export function startAdvisoryNotices(deps: Deps): () => void {
  return deps.subscribe('sync', (event, payload) => {
    if (event !== 'advisory_alert') return;
    deps.notify(ADVISORY_NOTICE_TITLE, advisoryNoticeBody(payload as AdvisoryAlert));
  });
}

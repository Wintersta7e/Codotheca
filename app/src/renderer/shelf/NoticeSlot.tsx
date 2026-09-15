import type { CSSProperties, ReactElement, ReactNode } from 'react';
import type { Notice } from './notice.js';
import { noticeAccent, noticeDismissKey, noticeIsDismissible, selectNotice } from './notice.js';

export const NOTICE_DISMISS_LABEL = 'DISMISS' as const;

export interface NoticeSlotProps {
  readonly candidates: readonly Notice[];
  readonly dismissed: readonly string[];
  readonly onDismiss: (key: string) => void;
  /**
   * The inside of the box, where the owning section has more than a `body` string to say.
   *
   * §8.0 owns the box, the priority and the dismissal key, and says geometry lives here and
   * only here; the sections it lists own what the box contains. Two of them cannot fit a
   * string: §1.4's identity card is a list of tickable rows with a statement of effect, and
   * §11.3a's residency ask has two answers whose dismissal must write a setting rather than a
   * `view_state` key. Returning non-null takes the body, the actions **and** the generic
   * `DISMISS` — the section is then responsible for every way out of its own row.
   */
  readonly renderContent?: (notice: Notice, dismiss: () => void) => ReactNode;
}

/** Block 2. Returns `null` — not an empty wrapper — when nothing qualifies: the wrapper's
 *  `padding:18px 22px 4px` would otherwise contribute 22px of dead band above block 3. */
export function NoticeSlot(props: NoticeSlotProps): ReactElement | null {
  const notice = selectNotice(props.candidates, props.dismissed);
  if (notice === null) return null;

  // §8.0: only the left border varies between priority 1 and everything else. The property is
  // `--cdt-` namespaced because `check-style-tokens.mjs` rejects any `var(--x)` a stylesheet
  // reads that is not declared in the token sheet, and this one is set per instance.
  const style = { '--cdt-notice-accent': `var(--${noticeAccent(notice.kind)})` } as CSSProperties;

  // The dismissal key is computed here and nowhere else, so a section owning its own way out
  // still writes the key §8.0 declared for it rather than a second spelling of one.
  const dismiss = (): void => {
    props.onDismiss(noticeDismissKey(notice.kind, notice.scope));
  };
  const supplied = props.renderContent?.(notice, dismiss) ?? null;
  if (supplied !== null) {
    return (
      <div className="cdt-shelf-notice-slot">
        <div className="cdt-shelf-notice" style={style} role="region" aria-label={notice.title}>
          {supplied}
        </div>
      </div>
    );
  }

  return (
    <div className="cdt-shelf-notice-slot">
      <div className="cdt-shelf-notice" style={style} role="region" aria-label={notice.title}>
        <p className="cdt-shelf-notice-title">{notice.title}</p>
        <p className="cdt-shelf-notice-body">{notice.body}</p>
        <div className="cdt-shelf-notice-actions">
          {notice.actions.map((action) => (
            <button
              key={action.label}
              type="button"
              className={`cdt-shelf-notice-${action.kind}`}
              onClick={action.run}
            >
              {action.label}
            </button>
          ))}
          {noticeIsDismissible(notice.kind) ? (
            <button
              type="button"
              className="cdt-shelf-notice-secondary"
              onClick={() => {
                props.onDismiss(noticeDismissKey(notice.kind, notice.scope));
              }}
            >
              {NOTICE_DISMISS_LABEL}
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}

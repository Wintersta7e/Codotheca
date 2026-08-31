/**
 * §8.5.4. `projects.setNote` has exactly one caller in phase 1 and this is it.
 *
 * Two consequences of §8.5.4 are load-bearing and both are rendered rather than assumed: a note
 * can rewrite the shelf row through §5.2's description chain, which the panel says out loud; and
 * §1.12 exports the note to the sidecar on clean shutdown and hourly, not on every change, so no
 * copy here promises durability on the keystroke.
 */
import { useEffect, useRef, useState, type ReactElement } from 'react';

import type { ProjectId, ProjectRow } from '../../../generated/protocol';
import { useProjectPageDeps } from '../deps';
import { cascadeDelay } from '../motion';

export const NOTE_LABEL = 'WHAT WAS I DOING HERE';

/** §8.5.4: the only thing saying the surface is writable, so it sits at `--text-3`, not below. */
export const NOTE_EMPTY = 'Empty. Click to leave yourself a note.';

export const NOTE_FEEDS_SHELF = 'THE FIRST LINE BECOMES THIS PROJECT’S SHELF DESCRIPTION';

/** §5.2: the note is third in the description chain, so it only shows through where 1 and 2 miss. */
export function feedsShelf(row: ProjectRow): boolean {
  return row.descriptionSource === 'note' || row.description === null;
}

/** The column is nullable, and an absent note is not an empty one. */
export function noteToWire(text: string): string | null {
  return text.trim() === '' ? null : text;
}

export interface NotePanelProps {
  projectId: ProjectId;
  row: ProjectRow;
  note: string | null;
  onChanged: () => void;
}

export function NotePanel({ projectId, row, note, onChanged }: NotePanelProps): ReactElement {
  const deps = useProjectPageDeps();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(note ?? '');
  const fieldRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    setDraft(note ?? '');
  }, [note]);
  useEffect(() => {
    if (editing) fieldRef.current?.focus();
  }, [editing]);

  const commit = (): void => {
    setEditing(false);
    const next = noteToWire(draft);
    if (next === note) return;
    deps
      .request('projects.setNote', { id: projectId, note: next })
      .then(onChanged)
      .catch(() => {
        // The write did not land, so the panel goes back to drawing the core's last answer
        // rather than showing text nothing has stored.
        setDraft(note ?? '');
      });
  };

  return (
    <section
      className="cp-note cp-rise"
      data-testid="cp-note"
      style={{ animationDelay: cascadeDelay(5) }}
    >
      <div className="cp-note-label" data-testid="cp-note-label">
        {NOTE_LABEL}
      </div>

      {editing ? (
        <textarea
          ref={fieldRef}
          className="cp-note-field"
          data-testid="cp-note-field"
          rows={3}
          value={draft}
          aria-label={NOTE_LABEL}
          onChange={(e) => {
            setDraft(e.target.value);
          }}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === 'Escape') {
              // The page's key table already returns null on a text-entry target (R45), so this
              // is the second lock rather than the only one: the field owns Escape wherever it
              // is mounted, not only under this page.
              e.stopPropagation();
              setDraft(note ?? '');
              setEditing(false);
            }
          }}
        />
      ) : note === null ? (
        <button
          type="button"
          className="cp-note-empty"
          data-testid="cp-note-empty"
          onClick={() => {
            setEditing(true);
          }}
        >
          {NOTE_EMPTY}
        </button>
      ) : (
        <div
          className="cp-note-text"
          data-testid="cp-note-text"
          role="button"
          tabIndex={0}
          onClick={() => {
            setEditing(true);
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter') setEditing(true);
          }}
        >
          {note}
        </div>
      )}

      {feedsShelf(row) ? (
        <div className="cp-note-feeds" data-testid="cp-note-feeds">
          {NOTE_FEEDS_SHELF}
        </div>
      ) : null}
    </section>
  );
}

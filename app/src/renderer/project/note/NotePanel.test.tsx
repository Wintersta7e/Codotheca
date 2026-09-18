import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ProjectId, ProjectRow } from '../../../generated/protocol';
import { ProjectPageDepsContext, type ProjectPageDeps } from '../deps';
import { NOW, rowFixture } from '../testFixtures';
import { feedsShelf, NOTE_EMPTY, NOTE_LABEL, NotePanel, noteToWire } from './NotePanel';

afterEach(cleanup);

const PROJECT = 7 as ProjectId;

interface Drawn {
  readonly request: ReturnType<typeof vi.fn>;
  readonly onChanged: ReturnType<typeof vi.fn>;
  readonly outerKeys: ReturnType<typeof vi.fn>;
}

function draw(note: string | null, row: ProjectRow = rowFixture()): Drawn {
  const request = vi.fn(() => Promise.resolve({}));
  const onChanged = vi.fn();
  const outerKeys = vi.fn();
  const deps: ProjectPageDeps = {
    request: request as unknown as ProjectPageDeps['request'],
    relocate: () => Promise.resolve({ kind: 'cancelled' }),
    uninstall: () => Promise.resolve({ kind: 'refused' as const, verdict: null }),
    openRemoteLink: () => Promise.resolve({ kind: 'not_linkable' }),
    subscribe: () => () => undefined,
    now: () => NOW,
  };
  render(
    // The page above this panel is where `Esc` would otherwise land, so the propagation guard
    // is asserted against a real ancestor listener rather than by spying on the event.
    <div onKeyDown={outerKeys}>
      <ProjectPageDepsContext.Provider value={deps}>
        <NotePanel projectId={PROJECT} row={row} note={note} onChanged={onChanged} />
      </ProjectPageDepsContext.Provider>
    </div>,
  );
  return { request, onChanged, outerKeys };
}

describe('the empty state', () => {
  it('invites the user in the label and the sentence §8.5.4 gives', () => {
    draw(null);
    expect(screen.getByTestId('cp-note-label').textContent).toBe(NOTE_LABEL);
    expect(screen.getByTestId('cp-note-empty').textContent).toBe(NOTE_EMPTY);
    expect(NOTE_EMPTY).toBe('Empty. Click to leave yourself a note.');
  });

  it('opens a field on click and gives it focus', () => {
    draw(null);
    fireEvent.click(screen.getByTestId('cp-note-empty'));
    expect(document.activeElement).toBe(screen.getByTestId('cp-note-field'));
  });
});

describe('writing', () => {
  it('sends the text on blur, scoped to this project', async () => {
    const { request, onChanged } = draw(null);
    fireEvent.click(screen.getByTestId('cp-note-empty'));
    const field = screen.getByTestId('cp-note-field');
    fireEvent.change(field, { target: { value: 'pick up the parser' } });
    fireEvent.blur(field);
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith('projects.setNote', {
        id: 7,
        note: 'pick up the parser',
      }),
    );
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it('clears to NULL and never to an empty string', async () => {
    expect(noteToWire('')).toBeNull();
    expect(noteToWire('   ')).toBeNull();
    expect(noteToWire(' kept ')).toBe(' kept ');
    const { request } = draw('something');
    fireEvent.click(screen.getByTestId('cp-note-text'));
    const field = screen.getByTestId('cp-note-field');
    fireEvent.change(field, { target: { value: '' } });
    fireEvent.blur(field);
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith('projects.setNote', { id: 7, note: null }),
    );
  });

  it('writes nothing when the text has not changed', async () => {
    const { request } = draw('unchanged');
    fireEvent.click(screen.getByTestId('cp-note-text'));
    fireEvent.blur(screen.getByTestId('cp-note-field'));
    await waitFor(() => expect(request).not.toHaveBeenCalled());
  });

  it('reverts on Escape and keeps the key away from the page', () => {
    const { request, outerKeys } = draw('original');
    fireEvent.click(screen.getByTestId('cp-note-text'));
    const field = screen.getByTestId('cp-note-field');
    fireEvent.change(field, { target: { value: 'discarded' } });
    fireEvent.keyDown(field, { key: 'Escape', code: 'Escape' });
    expect(screen.getByTestId('cp-note-text').textContent).toBe('original');
    expect(request).not.toHaveBeenCalled();
    expect(outerKeys).not.toHaveBeenCalled();
  });

  it('promises no durability it does not have', () => {
    draw('something');
    expect(screen.getByTestId('cp-note').textContent).not.toMatch(/saved|syncing|backed up/i);
  });
});

describe('the shelf warning', () => {
  it('is drawn exactly where the note would rewrite the tile', () => {
    expect(feedsShelf(rowFixture({ description: null, descriptionSource: null }))).toBe(true);
    expect(feedsShelf(rowFixture({ descriptionSource: 'note' }))).toBe(true);
    expect(
      feedsShelf(rowFixture({ description: 'from a manifest', descriptionSource: 'manifest' })),
    ).toBe(false);
    draw(null, rowFixture({ description: null, descriptionSource: null }));
    expect(screen.getByTestId('cp-note-feeds')).toBeTruthy();
  });

  it('is absent where the description comes from somewhere else', () => {
    draw(null, rowFixture({ description: 'from a manifest', descriptionSource: 'manifest' }));
    expect(screen.queryByTestId('cp-note-feeds')).toBeNull();
  });
});

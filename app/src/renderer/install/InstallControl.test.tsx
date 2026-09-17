import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';

import type { InstallPreview, Root, RootId } from '../../generated/protocol.js';
import { INSTALL_LABEL, InstallControl, REFUSAL_COPY, UPGRADE_LABEL } from './InstallControl.js';
import { ADD_A_FOLDER_LABEL, RootChooser } from './RootChooser.js';

afterEach(cleanup);

const preview = (over: Partial<InstallPreview> = {}): InstallPreview => ({
  destination: { rootId: 1 as RootId, seedBasename: 'widget', display: 'D:\\Work\\widget' },
  refusedBecause: null,
  ...over,
});

test('an uncomputed preview renders nothing — never a button over an unknown destination', () => {
  const { container } = render(
    <InstallControl preview={null} onInstall={vi.fn()} onOpenUpgrade={vi.fn()} />,
  );
  expect(container.textContent).toBe('');
});

test('it shows the composed display form the core sent, and never builds one', () => {
  render(<InstallControl preview={preview()} onInstall={vi.fn()} onOpenUpgrade={vi.fn()} />);
  expect(screen.getByText('D:\\Work\\widget')).toBeTruthy();
  expect(screen.getByRole('button', { name: INSTALL_LABEL })).toBeTruthy();
});

// AC-P2-24-23.
test('a private remote offers the upgrade and never says gone or missing', () => {
  const onOpenUpgrade = vi.fn();
  const onInstall = vi.fn();
  render(
    <InstallControl
      preview={preview({ destination: null, refusedBecause: 'private_needs_upgrade' })}
      onInstall={onInstall}
      onOpenUpgrade={onOpenUpgrade}
    />,
  );
  const text = document.body.textContent ?? '';
  for (const banned of ['gone', 'missing', 'Gone', 'Missing', 'does not exist', 'not found']) {
    expect(text).not.toContain(banned);
  }
  fireEvent.click(screen.getByRole('button', { name: UPGRADE_LABEL }));
  expect(onOpenUpgrade).toHaveBeenCalledTimes(1);
  expect(onInstall, 'it never auto-upgrades and never retries anonymously').not.toHaveBeenCalled();
});

test('every refusal has a sentence, and none of them offers to install anyway', () => {
  for (const refusal of Object.keys(REFUSAL_COPY) as (keyof typeof REFUSAL_COPY)[]) {
    cleanup();
    render(
      <InstallControl
        preview={preview({ destination: null, refusedBecause: refusal })}
        onInstall={vi.fn()}
        onOpenUpgrade={vi.fn()}
      />,
    );
    expect(screen.getByText(REFUSAL_COPY[refusal])).toBeTruthy();
    expect(screen.queryByRole('button', { name: INSTALL_LABEL })).toBeNull();
  }
});

const root = (id: number, path: string): Root =>
  ({ id: id as RootId, pathDisplay: path }) as unknown as Root;

test('the chooser sends a RootId and never a string path', () => {
  const onSelect = vi.fn();
  render(
    <RootChooser
      roots={[root(1, 'D:\\Work'), root(2, 'E:\\Shelf')]}
      selected={1 as RootId}
      onSelect={onSelect}
      onAddFolder={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole('button', { name: 'E:\\Shelf' }));
  expect(onSelect).toHaveBeenCalledWith(2);
  expect(onSelect).not.toHaveBeenCalledWith('E:\\Shelf');
});

test('ADD A FOLDER… invokes the shell dialog and nothing else', () => {
  const onAddFolder = vi.fn();
  const onSelect = vi.fn();
  render(<RootChooser roots={[]} selected={null} onSelect={onSelect} onAddFolder={onAddFolder} />);
  fireEvent.click(screen.getByRole('button', { name: ADD_A_FOLDER_LABEL }));
  expect(onAddFolder).toHaveBeenCalledTimes(1);
  expect(onSelect).not.toHaveBeenCalled();
});

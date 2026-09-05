import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { IdentityId, IdentityRow } from '../../generated/protocol.js';
import {
  DATA_GROUP_ROWS,
  DataGroups,
  GITHUB_CONSEQUENCE,
  NOT_IN_THIS_BUILD,
  bundleResultText,
  identityCaption,
  indexSizeText,
} from './groupsData.js';

afterEach(cleanup);

const props = {
  indexLocation: null,
  bundle: null,
  showRealPaths: false,
  identities: null,
  slots: {},
  onReveal: vi.fn(),
  onShowRealPaths: vi.fn(),
  onExport: vi.fn(),
};

const identity = (isUser: boolean, id: number): IdentityRow => ({
  id: id as unknown as IdentityId,
  email: `someone-${String(id)}@example.invalid`,
  name: null,
  isUser,
  source: 'gitconfig',
  aliasReason: null,
  primaryEmail: null,
  repositories: null,
  commits: 0,
  projects: 0,
  confirmedAt: null,
});

describe('group 8, DATA', () => {
  it('never renders an unknown index size as zero bytes', () => {
    expect(indexSizeText(null)).toBe('—');
    expect(indexSizeText({ pathDisplay: '<data>/index.db', sizeBytes: 4_194_304 })).toContain('4');
    render(<DataGroups {...props} />);
    expect(screen.queryByText(/\b0 B\b/)).toBeNull();
    expect(screen.getByText('—')).toBeTruthy();
  });

  it('reveals by naming a target, never by handing the shell a path', () => {
    const onReveal = vi.fn();
    render(
      <DataGroups
        {...props}
        indexLocation={{ pathDisplay: '<data>/index.db', sizeBytes: 1 }}
        onReveal={onReveal}
      />,
    );
    fireEvent.click(screen.getByRole('button', { name: 'REVEAL' }));
    expect(onReveal).toHaveBeenCalledWith('index');
  });

  it('exports anonymised by default, with SHOW REAL PATHS as the explicit toggle', () => {
    const onShowRealPaths = vi.fn();
    render(<DataGroups {...props} onShowRealPaths={onShowRealPaths} />);
    const toggle = screen.getByRole('switch', { name: 'SHOW REAL PATHS' });
    // §11.4 anonymises by default and one click reveals.
    expect(toggle.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(toggle);
    expect(onShowRealPaths).toHaveBeenCalledWith(true);
  });

  it('says which state the last export was written in, and nothing before there is one', () => {
    expect(bundleResultText(null)).toBeNull();
    render(
      <DataGroups
        {...props}
        bundle={{
          pathBytes: { b64: '' },
          pathDisplay: '<data>/bundle.json',
          sizeBytes: 2048,
          anonymised: true,
        }}
      />,
    );
    expect(screen.getByText(/ANONYMISED/)).toBeTruthy();
  });

  it('draws HIDE A PROJECT only when a picker exists behind CHOOSE', () => {
    const chooseProjectToHide = vi.fn();
    const { rerender } = render(<DataGroups {...props} />);
    expect(screen.queryByRole('button', { name: 'CHOOSE' })).toBeNull();
    expect(screen.queryByText('HIDE A PROJECT')).toBeNull();
    rerender(<DataGroups {...props} slots={{ chooseProjectToHide }} />);
    fireEvent.click(screen.getByRole('button', { name: 'CHOOSE' }));
    expect(chooseProjectToHide).toHaveBeenCalledTimes(1);
    expect(
      screen.getByText('Keeps it off the shelf and out of every count. Nothing is removed.'),
    ).toBeTruthy();
  });

  it('offers no control that destroys anything, in any spelling', () => {
    render(<DataGroups {...props} slots={{ chooseProjectToHide: vi.fn() }} />);
    // Criterion 44, at the one surface where such a control would most plausibly be added.
    for (const banned of [/\bforget\b/i, /\buninstall\b/i, /\bdelete\b/i, /\bremove\b/i]) {
      for (const control of [...screen.getAllByRole('button'), ...screen.getAllByRole('switch')]) {
        const name = control.getAttribute('aria-label') ?? control.textContent ?? '';
        expect(name).not.toMatch(banned);
      }
    }
    for (const spec of DATA_GROUP_ROWS) {
      expect(`${spec.id} ${spec.label}`.toUpperCase()).not.toContain('FORGET');
    }
  });
});

describe('group 8a, IDENTITY', () => {
  it('is not drawn at all without the card it re-opens', () => {
    render(<DataGroups {...props} identities={[identity(true, 1)]} />);
    expect(screen.queryByRole('group', { name: /IDENTITY/ })).toBeNull();
  });

  it('with the card, captions itself from the confirmed count', () => {
    render(
      <DataGroups
        {...props}
        slots={{ identityCard: () => null, addIdentityAddress: vi.fn() }}
        identities={[identity(true, 1), identity(true, 2), identity(false, 3)]}
      />,
    );
    expect(screen.getByRole('group', { name: 'IDENTITY' })).toBeTruthy();
    expect(screen.getByText('2 ADDRESSES ARE YOURS')).toBeTruthy();
  });

  it('never captions an unread identity list as zero addresses', () => {
    expect(identityCaption(null)).toBe('—');
    expect(identityCaption([])).toBe('0 ADDRESSES ARE YOURS');
    render(<DataGroups {...props} slots={{ identityCard: () => null }} />);
    expect(screen.queryByText(/0 ADDRESSES/)).toBeNull();
  });

  it('offers + ADD AN ADDRESS only when something can widen the set', () => {
    const addIdentityAddress = vi.fn();
    const { rerender } = render(<DataGroups {...props} slots={{ identityCard: () => null }} />);
    expect(screen.queryByRole('button', { name: '+ ADD AN ADDRESS' })).toBeNull();
    rerender(<DataGroups {...props} slots={{ identityCard: () => null, addIdentityAddress }} />);
    fireEvent.click(screen.getByRole('button', { name: '+ ADD AN ADDRESS' }));
    expect(addIdentityAddress).toHaveBeenCalledTimes(1);
  });
});

describe('group 9, GITHUB, and the notification block', () => {
  it('keeps the consequence verbatim and offers no control', () => {
    const { container } = render(<DataGroups {...props} />);
    expect(screen.getByText('NOT CONNECTED')).toBeTruthy();
    expect(screen.getByText(GITHUB_CONSEQUENCE)).toBeTruthy();
    expect(GITHUB_CONSEQUENCE).toContain('unknown is drawn as unknown, never as zero');
    // CONNECT GITHUB opens nothing in phase 1, so it is cut and its line stands.
    expect(screen.queryByRole('button', { name: /CONNECT/i })).toBeNull();
    // [p2] §20.3: the three chips are deleted. Two of the three named strings that are not
    // GitHub OAuth scopes at all, and the third was a promise about what is absent. A granted
    // scope is a read-back fact and is rendered only in the CONNECTED state, from the payload.
    expect(container.querySelectorAll('[data-row="github-statement"] span')).toHaveLength(1);
  });

  it('marks all three notifications NOT IN THIS BUILD and draws none as a control', () => {
    const { container } = render(<DataGroups {...props} />);
    expect(screen.getAllByText(new RegExp(NOT_IN_THIS_BUILD))).toHaveLength(3);
    const rows = DATA_GROUP_ROWS.filter((r) => r.group === 'notifications');
    expect(rows).toHaveLength(3);
    expect(rows.every((r) => r.backing.kind === 'statement')).toBe(true);
    for (const row of rows) {
      const drawn = container.querySelector(`[data-row="${row.id}"]`);
      expect(drawn?.querySelector('button, [role="switch"], [tabindex]')).toBeNull();
    }
  });

  it('keeps the footer verbatim, both lines', () => {
    render(<DataGroups {...props} />);
    expect(screen.getByText('CODOTHECA · GPL-3.0 · NO ACCOUNT · NO TELEMETRY')).toBeTruthy();
    expect(screen.getByText('THREE NOTIFICATIONS EXIST · NONE MENTIONS ABSENCE')).toBeTruthy();
  });
});

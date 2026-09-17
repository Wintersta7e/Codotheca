import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type {
  UninstallBlocker,
  UninstallDisposition,
  UninstallVerdict,
} from '../../../generated/protocol';
import { UninstallControl } from './UninstallControl';
import { blockerSentence, UNINSTALL_CONFIRMATION, UNINSTALL_LABEL } from './uninstallCopy';

afterEach(cleanup);

/** All fourteen, so the loops below cannot silently cover thirteen. */
const ALL_BLOCKERS: UninstallBlocker[] = [
  'unpushed_commits',
  'uncommitted_changes',
  'stash_present',
  'untracked_precious',
  'ignored_precious',
  'submodule_unsafe',
  'linked_worktree',
  'shallow_clone',
  'remote_unreachable',
  'remote_is_local_mirror',
  'stash_unreadable',
  'live_session',
  'refused_path',
  'never_observed',
];

const UNKNOWN_CLASS: UninstallBlocker[] = [
  'shallow_clone',
  'remote_unreachable',
  'stash_unreadable',
  'never_observed',
];

const verdict = (over: Partial<UninstallVerdict> = {}): UninstallVerdict => ({
  disposition: 'safe',
  blockers: [],
  remoteVerifiedAt: 1_700_000_000,
  trashAvailable: true,
  computedAt: 1_700_000_000,
  ...over,
});

/**
 * **The assertion that matters.** Not "the button is disabled" — a test that only checks the
 * visible button passes against a product with a context-menu override. This asks the DOM whether
 * **anything at all** could reach the removal.
 */
function reachableActivators(): Element[] {
  const selectors = [
    'button',
    'a[href]',
    '[role="button"]',
    '[role="menuitem"]',
    '[onclick]',
    '[tabindex]:not([tabindex="-1"])',
    'input[type="submit"]',
    'input[type="button"]',
  ];
  return [...document.querySelectorAll(selectors.join(','))];
}

describe('the uninstall control', () => {
  it('names every one of the fourteen blockers when it is blocked', () => {
    for (const blocker of ALL_BLOCKERS) {
      cleanup();
      render(
        <UninstallControl
          verdict={verdict({
            disposition: UNKNOWN_CLASS.includes(blocker) ? 'unknown' : 'blocked',
            blockers: [blocker],
          })}
          onUninstall={vi.fn()}
        />,
      );
      expect(screen.getByText(blockerSentence(blocker)), blocker).toBeTruthy();
      expect(reachableActivators(), `${blocker} must reach nothing`).toHaveLength(0);
    }
  });

  it('names every blocker when there are several, not just the first', () => {
    render(
      <UninstallControl
        verdict={verdict({
          disposition: 'blocked',
          blockers: ['unpushed_commits', 'stash_present', 'live_session'],
        })}
        onUninstall={vi.fn()}
      />,
    );
    for (const blocker of ['unpushed_commits', 'stash_present', 'live_session'] as const) {
      expect(screen.getByText(blockerSentence(blocker))).toBeTruthy();
    }
  });

  /** AC-P2-24-14: three states, not two. */
  it('renders unknown distinctly from blocked', () => {
    const { container: blocked } = render(
      <UninstallControl
        verdict={verdict({ disposition: 'blocked', blockers: ['unpushed_commits'] })}
        onUninstall={vi.fn()}
      />,
    );
    const blockedState = blocked.querySelector('.cp-uninstall')?.getAttribute('data-state');
    const blockedText = blocked.textContent ?? '';
    cleanup();

    const { container: unknown } = render(
      <UninstallControl
        verdict={verdict({ disposition: 'unknown', blockers: ['remote_unreachable'] })}
        onUninstall={vi.fn()}
      />,
    );
    const unknownState = unknown.querySelector('.cp-uninstall')?.getAttribute('data-state');
    const unknownText = unknown.textContent ?? '';

    expect(blockedState).toBe('blocked');
    expect(unknownState).toBe('unknown');
    expect(
      unknownText,
      'unknown is *I could not check*; blocked is *I checked and the answer is no*',
    ).not.toBe(blockedText);
  });

  it('offers the removal only when the verdict is safe, with the confirmation copy', () => {
    render(<UninstallControl verdict={verdict()} onUninstall={vi.fn()} />);
    expect(screen.getByText(UNINSTALL_CONFIRMATION)).toBeTruthy();
    expect(screen.getByRole('button', { name: UNINSTALL_LABEL })).toBeTruthy();
  });

  it('says what will happen to the copy before the click', () => {
    render(<UninstallControl verdict={verdict({ trashAvailable: true })} onUninstall={vi.fn()} />);
    expect(document.body.textContent).toContain('recycle bin');
    cleanup();
    render(<UninstallControl verdict={verdict({ trashAvailable: false })} onUninstall={vi.fn()} />);
    expect(document.body.textContent).toContain('outright');
  });

  it('is disabled while the verdict is in flight, and never enabled-then-disabled', () => {
    render(<UninstallControl verdict={null} onUninstall={vi.fn()} />);
    expect(reachableActivators()).toHaveLength(0);
  });

  /** Phase 2 ships none of these words anywhere in the control. */
  it('renders no removal wording and no override', () => {
    for (const disposition of ['safe', 'blocked', 'unknown'] as UninstallDisposition[]) {
      cleanup();
      render(
        <UninstallControl
          verdict={verdict({
            disposition,
            blockers: disposition === 'safe' ? [] : ['refused_path'],
          })}
          onUninstall={vi.fn()}
        />,
      );
      const text = document.body.textContent ?? '';
      for (const banned of ['DELETE', 'REMOVE THE', 'RECLAIM SPACE', 'anyway', 'Anyway', 'Force']) {
        expect(text, `${disposition} must not say ${banned}`).not.toContain(banned);
      }
    }
  });
});

/** §24.5: no bulk path exists. One copy, one decision. */
describe('the panel offers no bulk selection', () => {
  it('has no multi-select, exclusion summary or type-the-count gate', async () => {
    const sources = await Promise.all([
      import('./uninstallCopy?raw'),
      import('./UninstallControl.tsx?raw'),
    ]);
    for (const module of sources) {
      const text = module.default;
      expect(text.length, 'the source must actually have been read').toBeGreaterThan(100);
      for (const banned of ['checkbox', 'selectAll', 'multiSelect', 'type the number']) {
        expect(text).not.toContain(banned);
      }
    }
  });
});

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

// `?raw` rather than node:fs: the renderer project carries no Node types by design.
import schemaRaw from '../../../../../protocol/schema/protocol.json?raw';
import type {
  TrashRefusalKind,
  UninstallBlocker,
  UninstallDisposition,
  UninstallVerdict,
} from '../../../generated/protocol';
import { UninstallControl } from './UninstallControl';
import {
  blockerSentence,
  TRASH_AVAILABLE_NOTE,
  trashRefusalSentence,
  UNINSTALL_CONFIRMATION,
  UNINSTALL_LABEL,
} from './uninstallCopy';

afterEach(cleanup);

/**
 * Every blocker **the schema declares**, read from the tracked schema both languages are generated
 * from. A hand list here was the fourth copy of one count (§45.9, R201): it said fourteen after
 * the schema said more, and every loop below would have covered the old set and passed.
 */
function schemaBlockers(): UninstallBlocker[] {
  expect(schemaRaw.length, 'protocol.json read as an empty string').toBeGreaterThan(0);
  const raw = JSON.parse(schemaRaw) as { types?: Record<string, { variants?: string[] }> };
  return (raw.types?.['UninstallBlocker']?.variants ?? []) as UninstallBlocker[];
}

/** Every reason the bin can refuse a copy, read from the same schema (§46.7). */
function schemaTrashRefusals(): TrashRefusalKind[] {
  expect(schemaRaw.length, 'protocol.json read as an empty string').toBeGreaterThan(0);
  const raw = JSON.parse(schemaRaw) as { types?: Record<string, { variants?: string[] }> };
  return (raw.types?.['TrashRefusalKind']?.variants ?? []) as TrashRefusalKind[];
}

const verdict = (over: Partial<UninstallVerdict> = {}): UninstallVerdict => ({
  disposition: 'safe',
  blockers: [],
  remoteVerifiedAt: 1_700_000_000,
  trashAvailable: true,
  computedAt: 1_700_000_000,
  nested: [],
  precious: null,
  trashRefusal: null,
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
  it('names every blocker the schema declares when it is blocked', () => {
    const all = schemaBlockers();
    let covered = 0;
    for (const blocker of all) {
      // Both dispositions: the renderer never classifies, so each sentence must render under the
      // one the core would choose whichever class the core puts it in.
      for (const disposition of ['blocked', 'unknown'] as const) {
        cleanup();
        render(
          <UninstallControl
            verdict={verdict({ disposition, blockers: [blocker] })}
            onUninstall={vi.fn()}
          />,
        );
        const sentence = blockerSentence(blocker) as string | undefined;
        expect(
          sentence ?? '',
          `schema declares ${String(all.length)}, sentence missing for ${blocker}`,
        ).not.toBe('');
        expect(screen.getByText(blockerSentence(blocker)), blocker).toBeTruthy();
        expect(reachableActivators(), `${blocker} must reach nothing`).toHaveLength(0);
      }
      covered += 1;
    }
    console.warn(`uninstall control: ${String(covered)} / ${String(all.length)} blockers named`);
    expect(covered, 'the schema declares zero blockers, so this asserts nothing').toBeGreaterThan(
      0,
    );
    expect(covered).toBe(all.length);
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
    const blockedText = blocked.textContent;
    cleanup();

    const { container: unknown } = render(
      <UninstallControl
        verdict={verdict({ disposition: 'unknown', blockers: ['remote_unreachable'] })}
        onUninstall={vi.fn()}
      />,
    );
    const unknownState = unknown.querySelector('.cp-uninstall')?.getAttribute('data-state');
    const unknownText = unknown.textContent;

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
    expect(document.body.textContent).toContain(TRASH_AVAILABLE_NOTE);
    expect(screen.getByRole('button', { name: UNINSTALL_LABEL })).toBeTruthy();
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
      const text = document.body.textContent;
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

/**
 * **AC-P4-46-13's control half (§46.7's interim rule).** For every reason the schema declares, a
 * `safe` verdict whose bin cannot take the copy renders that reason and **nothing that could
 * reach the removal**; with no reason, the recycle-bin note and the button.
 */
it('AC-P4-46-13-control', () => {
  const kinds = schemaTrashRefusals();
  let covered = 0;
  for (const kind of kinds) {
    cleanup();
    render(
      <UninstallControl
        verdict={verdict({ trashAvailable: false, trashRefusal: kind })}
        onUninstall={vi.fn()}
      />,
    );
    expect(document.body.textContent, kind).toContain(trashRefusalSentence(kind));
    expect(reachableActivators(), `${kind} rendered an activator`).toHaveLength(0);
    covered += 1;
  }
  cleanup();
  render(<UninstallControl verdict={verdict()} onUninstall={vi.fn()} />);
  expect(document.body.textContent).toContain(TRASH_AVAILABLE_NOTE);
  expect(screen.getByRole('button', { name: UNINSTALL_LABEL })).toBeTruthy();
  expect(
    covered,
    'the schema declares zero trash refusals, so this asserts nothing',
  ).toBeGreaterThan(0);
});

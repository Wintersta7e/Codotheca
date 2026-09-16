import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { CiRun, ProjectId, RemoteFacts } from '../../../generated/protocol';
import { remoteFactsFixture } from '../testFixtures';
import { ciInk, ciLabel } from './ciCopy';
import { RemoteTab } from './RemoteTab';

afterEach(cleanup);

const NOW = 1_800_000_000;
const PROJECT = 7 as unknown as ProjectId;

function run(over: Partial<CiRun> = {}): CiRun {
  return {
    runId: 1,
    workflow: 'ci',
    conclusion: 'success',
    branch: 'main',
    runNumber: 1,
    startedAt: NOW - 600,
    ...over,
  };
}

function draw(over: Partial<RemoteFacts>): void {
  render(
    <RemoteTab
      projectId={PROJECT}
      facts={remoteFactsFixture({ state: 'observed', observedAt: NOW - 600, ...over })}
      shown={null}
      now={NOW}
      onOpenLink={vi.fn()}
    />,
  );
}

function rows(): HTMLElement[] {
  return [...screen.getByTestId('cp-remote-ci').querySelectorAll('.cp-remote-ci-row')].map(
    (node) => node as HTMLElement,
  );
}

describe('AC-P2-25-6 the CI list renders the record and never judges it', () => {
  it('renders at most five runs out of seven', () => {
    const seven = Array.from({ length: 7 }, (_, i) => run({ runId: i + 1, runNumber: i + 1 }));
    draw({ ci: { state: 'observed', runs: seven, observedAt: NOW - 300 } });
    expect(rows()).toHaveLength(5);
  });

  it('prints the branch exactly once per row', () => {
    draw({
      ci: { state: 'observed', runs: [run({ branch: 'release-1' })], observedAt: NOW - 300 },
    });
    const text = rows()[0]?.textContent ?? '';
    // Counted, not merely asserted present: the screenshot prints the branch twice and §25.4
    // rules once, so a second copy must fail rather than read as the same assertion passing.
    expect(text.match(/release-1/gu)).toHaveLength(1);
    expect(text).toContain('run #1');
  });

  it('maps every conclusion §25.4 names, and its ink token', () => {
    const cases: readonly [string | null, string, string][] = [
      ['success', 'PASSED', '--pass'],
      ['failure', 'FAILED', '--fail'],
      ['cancelled', 'CANCELLED', '--warn'],
      ['timed_out', 'TIMED OUT', '--warn'],
      ['action_required', 'ACTION REQUIRED', '--warn'],
      ['neutral', 'NEUTRAL', '--text-3'],
      ['skipped', 'SKIPPED', '--text-3'],
      ['startup_failure', 'STARTUP FAILURE', '--text-3'],
      [null, 'RUNNING', '--text-3'],
    ];
    for (const [stored, label, ink] of cases) {
      expect(ciLabel(stored), String(stored)).toBe(label);
      expect(ciInk(stored), String(stored)).toBe(ink);
    }
  });

  it('renders an unrecognised conclusion uppercased and verbatim', () => {
    expect(ciLabel('whatever_the_forge_invents_next')).toBe('WHATEVER_THE_FORGE_INVENTS_NEXT');
    draw({
      ci: {
        state: 'observed',
        runs: [run({ conclusion: 'whatever_the_forge_invents_next' })],
        observedAt: NOW - 300,
      },
    });
    expect(rows()[0]?.textContent).toContain('WHATEVER_THE_FORGE_INVENTS_NEXT');
  });

  it('renders a NULL conclusion as RUNNING', () => {
    draw({ ci: { state: 'observed', runs: [run({ conclusion: null })], observedAt: NOW - 300 } });
    expect(rows()[0]?.textContent).toContain('RUNNING');
  });

  /**
   * There is no "CI green" boolean anywhere in phase 2 — the aggregate *is* the check, and the
   * check is phase 3. Asserted over the rendered output as well as over the module's exports,
   * because a boolean that never rendered would still be an aggregate somebody could read.
   */
  it('renders no aggregate over the run set', () => {
    draw({
      ci: {
        state: 'observed',
        runs: [run({ runId: 1, conclusion: 'success' }), run({ runId: 2, conclusion: 'failure' })],
        observedAt: NOW - 300,
      },
    });
    const text = screen.getByTestId('cp-remote-ci').textContent ?? '';
    for (const banned of ['CI GREEN', 'ALL PASSING', 'ALL PASSED', 'HEALTHY', '1 OF 2', '2 RUNS']) {
      expect(text, banned).not.toContain(banned);
    }
  });
});

describe('§25.1 the CI list carries its own clock and its own state', () => {
  it('renders two different ages when the two reads happened at different instants', () => {
    draw({
      observedAt: NOW - 600,
      ci: { state: 'observed', runs: [run()], observedAt: NOW - 7200 },
    });
    expect(screen.getByTestId('cp-remote-observed').textContent).toBe('OBSERVED 10m');
    expect(screen.getByTestId('cp-remote-ci-observed').textContent).toBe('OBSERVED 2h');
  });

  it('renders the list as not permitted while the three forge blocks still render numbers', () => {
    draw({
      stars: 41,
      ci: { state: 'not_permitted', runs: [], observedAt: null },
    });
    expect(screen.getByTestId('cp-remote-ci-state').textContent).toBe('NOT PERMITTED');
    expect(screen.getByTestId('cp-remote-stars').textContent).toContain('41');
  });

  it('renders no list at all with no account connected', () => {
    render(
      <RemoteTab
        projectId={PROJECT}
        facts={remoteFactsFixture({
          state: 'no_account',
          ci: { state: 'no_account', runs: [], observedAt: null },
        })}
        shown={null}
        now={NOW}
        onOpenLink={vi.fn()}
      />,
    );
    expect(screen.queryByTestId('cp-remote-ci')).toBeNull();
  });
});

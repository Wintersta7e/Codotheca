import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { LocationDetail, ProjectId, RemoteFacts } from '../../../generated/protocol';
import { locationFixture, remoteFactsFixture } from '../testFixtures';
import { RemoteTab } from './RemoteTab';

afterEach(cleanup);

const NOW = 1_800_000_000;
const PROJECT = 7 as unknown as ProjectId;

function draw(
  over: Partial<RemoteFacts> = {},
  shown: LocationDetail | null = null,
): { readonly onOpenLink: ReturnType<typeof vi.fn> } {
  const onOpenLink = vi.fn();
  render(
    <RemoteTab
      projectId={PROJECT}
      facts={remoteFactsFixture(over)}
      shown={shown}
      now={NOW}
      onOpenLink={onOpenLink}
    />,
  );
  return { onOpenLink };
}

function pageText(): string {
  return screen.getByTestId('cp-remote').textContent ?? '';
}

function blockText(label: string): string {
  return screen.getByTestId(`cp-remote-${label}`).textContent ?? '';
}

describe('AC-P2-25-2 with no account the forge blocks and the CI list do not render', () => {
  it('draws the header, BEHIND and the links row, and nothing that needs a token', () => {
    draw(
      { state: 'no_account', ci: { state: 'no_account', runs: [], observedAt: null } },
      locationFixture({ behind: 3 }),
    );

    expect(screen.getByTestId('cp-remote-key').textContent).toBe('github.com/acme/widget');
    expect(screen.getByTestId('cp-remote-links')).toBeTruthy();
    expect(screen.getByTestId('cp-remote-behind')).toBeTruthy();
    expect(screen.queryByTestId('cp-remote-open-issues')).toBeNull();
    expect(screen.queryByTestId('cp-remote-open-prs')).toBeNull();
    expect(screen.queryByTestId('cp-remote-stars')).toBeNull();
    expect(screen.queryByTestId('cp-remote-ci')).toBeNull();
    // One statement, once per surface.
    expect(screen.getByTestId('cp-remote-no-account')).toBeTruthy();
  });

  it('renders the string UNKNOWN zero times', () => {
    draw({ state: 'no_account', ci: { state: 'no_account', runs: [], observedAt: null } });
    expect(pageText().match(/UNKNOWN/gu)).toBeNull();
  });
});

describe('AC-P2-25-3 an unobserved value renders the glyph and never a zero', () => {
  it('renders — and NOT YET FETCHED for every forge slot, and 0 for none', () => {
    draw();
    for (const label of ['open-issues', 'open-prs', 'stars']) {
      expect(blockText(label)).toContain('—');
      expect(blockText(label)).toContain('NOT YET FETCHED');
      expect(blockText(label)).not.toMatch(/\b0\b/u);
    }
  });

  it('renders — and NOT PERMITTED rather than an absent block for a repository the token may not see', () => {
    draw({
      state: 'not_permitted',
      ci: { state: 'not_permitted', runs: [], observedAt: null },
    });
    expect(blockText('stars')).toContain('—');
    expect(blockText('stars')).toContain('NOT PERMITTED');
    expect(screen.getByTestId('cp-remote-ci-state').textContent).toBe('NOT PERMITTED');
  });
});

describe('AC-P2-25-4 a measured zero and an unobserved value are different renders', () => {
  it('renders 0 with its meaning for a measured zero', () => {
    draw({ state: 'observed', observedAt: NOW - 60, openPrs: 0, openPrsFromUser: 0 });
    expect(blockText('open-prs')).toContain('0');
    expect(blockText('open-prs')).toContain('none from you');
    expect(blockText('open-prs')).not.toContain('—');
  });

  it('renders — for an unobserved value inside an observed read', () => {
    draw({ state: 'observed', observedAt: NOW - 60, stars: 41, openIssues: null });
    expect(blockText('stars')).toContain('41');
    expect(blockText('open-issues')).toContain('—');
    expect(blockText('open-issues')).toContain('NOT YET FETCHED');
  });
});

describe('§25.1 the sub-lines, and the two blocks differ deliberately', () => {
  it('renders the good-first-issue sub-line only above zero', () => {
    draw({ state: 'observed', observedAt: NOW - 60, openIssues: 7, goodFirstIssues: 3 });
    expect(blockText('open-issues')).toContain('3 labelled good-first-issue');
  });

  it('renders no issues sub-line at a measured zero, while the value slot still renders it', () => {
    draw({ state: 'observed', observedAt: NOW - 60, openIssues: 7, goodFirstIssues: 0 });
    expect(blockText('open-issues')).toContain('7');
    expect(blockText('open-issues')).not.toContain('good-first-issue');
  });

  it('renders the PR sub-line AT a measured zero — present, not absent', () => {
    draw({ state: 'observed', observedAt: NOW - 60, openPrs: 4, openPrsFromUser: 0 });
    expect(blockText('open-prs')).toContain('none from you');
    cleanup();
    draw({ state: 'observed', observedAt: NOW - 60, openPrs: 4, openPrsFromUser: 2 });
    expect(blockText('open-prs')).toContain('2 from you');
  });

  it('renders no sub-line at all from a NULL, never one built from a zero', () => {
    draw({ state: 'observed', observedAt: NOW - 60, openIssues: 7, goodFirstIssues: null });
    expect(blockText('open-issues')).not.toContain('good-first-issue');
    cleanup();
    draw({ state: 'observed', observedAt: NOW - 60, openPrs: 4, openPrsFromUser: null });
    expect(blockText('open-prs')).not.toContain('from you');
  });

  it('renders no star delta, because a delta cannot be computed from a first observation', () => {
    draw({ state: 'observed', observedAt: NOW - 60, stars: 41 });
    expect(blockText('stars')).toContain('41');
    expect(blockText('stars')).not.toContain('+');
    expect(blockText('stars')).not.toContain('this month');
  });
});

describe('§25.1 the observation line, and staleness is a function of now', () => {
  it('renders OBSERVED with an age inside the threshold', () => {
    draw({ state: 'observed', observedAt: NOW - 600 });
    expect(screen.getByTestId('cp-remote-observed').textContent).toBe('OBSERVED 10m');
  });

  it('renders stale past the threshold, with the number still on the block', () => {
    draw({ state: 'observed', observedAt: NOW - 86_400, stars: 41 });
    expect(screen.getByTestId('cp-remote-observed').textContent).toBe('stale · 1d');
    expect(blockText('stars')).toContain('41');
  });
});

describe('AC-P2-25-26 the fork line renders as text and carries no link', () => {
  it('renders FORK OF only when the parent key is observed', () => {
    draw({ state: 'observed', observedAt: NOW - 60 });
    expect(screen.queryByTestId('cp-remote-fork')).toBeNull();
    cleanup();

    draw({
      state: 'observed',
      observedAt: NOW - 60,
      forkParentKey: 'github.com/upstream/widget',
    });
    const fork = screen.getByTestId('cp-remote-fork');
    expect(fork.textContent).toBe('FORK OF github.com/upstream/widget');
    expect(fork.querySelector('a')).toBeNull();
    expect(fork.querySelector('button')).toBeNull();
  });
});

describe('AC-P2-25-12 an unlinkable key draws no links row and no link affordance', () => {
  it('draws the key as text and the row not at all', () => {
    draw({ linkable: false, key: 'forge.example.invalid/acme/widget' });
    expect(screen.queryByTestId('cp-remote-links')).toBeNull();
    const key = screen.getByTestId('cp-remote-key');
    expect(key.tagName).toBe('SPAN');
    expect(key.querySelector('a')).toBeNull();
  });

  it('sends an id and a kind, never a URL', () => {
    const { onOpenLink } = draw({ linkable: true });
    screen.getByRole('button', { name: 'ISSUES' }).click();
    expect(onOpenLink).toHaveBeenCalledWith(PROJECT, 'issues');
    expect(onOpenLink.mock.calls[0]).toHaveLength(2);
  });
});

describe('AC-P2-25-5 BEHIND is the local figure, through §8.5.2s own producer', () => {
  it('renders a measured zero with its meaning and the fetch age', () => {
    draw({}, locationFixture({ behind: 0, fetchHeadAt: NOW - 3600 }));
    expect(blockText('behind')).toContain('0');
    expect(blockText('behind')).toContain('no known divergence');
    expect(blockText('behind')).toContain('last fetch 1h');
  });

  it('says so when no fetch has been recorded', () => {
    draw({}, locationFixture({ behind: 2, fetchHeadAt: null }));
    expect(blockText('behind')).toContain('2');
    expect(blockText('behind')).toContain('no fetch recorded');
  });

  it('renders the glyph rather than a zero when nothing has compared this copy', () => {
    draw({}, locationFixture({ behind: null, fetchHeadAt: null }));
    expect(blockText('behind')).toContain('—');
  });

  // The import half of this criterion is asserted in `app/test/remoteScope.test.ts`, the node
  // project: this file runs in jsdom, where `import.meta.url` is not a file URL and a source
  // read is not expressible there.
});

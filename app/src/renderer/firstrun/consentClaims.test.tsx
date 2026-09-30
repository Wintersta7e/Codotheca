/**
 * **AC-P4-48-5** (§48.3 rows 1 and 2): the first-run standfirst and consent paragraph, as the
 * production screen renders them, claim nothing never leaves the machine and name what the
 * unauthenticated advisory lookup carries.
 *
 * The methods that send no credential are not listed here. They are read from the core's own
 * provider census through the derivation `npm run check:egress` runs, so a new unauthenticated
 * destination fails this test until the consent copy names it.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, test, vi } from 'vitest';
import modRs from '../../../../core/src/provider/mod.rs?raw';
import githubRs from '../../../../core/src/provider/github.rs?raw';
import { CENSUS_CATEGORIES, providerCensus } from '../../../../scripts/check-egress-census.mjs';
import { required } from '../../shared/required';
import { RootsScreen } from './RootsScreen';
import { WHAT_EXACTLY_LABEL } from './copy';

afterEach(cleanup);

/** The standfirst under the headline, and the paragraph WHAT EXACTLY opens, as rendered text. */
function renderedClaims(): { standfirst: string; paragraph: string } {
  render(
    <RootsScreen
      deps={{
        onToggleRoot: vi.fn(),
        onConsent: vi.fn(),
        onAddFolder: vi.fn(),
        onConfirmLarge: vi.fn(),
        onDig: vi.fn(),
      }}
      rows={[]}
      ticked={new Set()}
      consented
      pendingConfirm={null}
      tier="full"
      busy={false}
    />,
  );
  const standfirst = required(
    screen.getByRole('heading', { level: 1 }).nextElementSibling,
    'the element under the headline',
  );
  fireEvent.click(screen.getByRole('button', { name: WHAT_EXACTLY_LABEL }));
  const paragraph = required(
    screen.getByRole('button', { name: WHAT_EXACTLY_LABEL }).nextElementSibling,
    'the paragraph WHAT EXACTLY opens',
  );
  return { standfirst: standfirst.textContent, paragraph: paragraph.textContent };
}

test('AC-P4-48-5 the rendered standfirst and consent paragraph claim nothing never leaves and name the unauthenticated lookup', () => {
  // An empty `?raw` would make the census find nothing and every loop below pass on nothing.
  expect(modRs.length, 'core/src/provider/mod.rs read as empty').toBeGreaterThan(0);
  expect(githubRs.length, 'core/src/provider/github.rs read as empty').toBeGreaterThan(0);

  const { standfirst, paragraph } = renderedClaims();
  expect(standfirst, 'the standfirst rendered no text').not.toBe('');
  expect(paragraph, 'the consent paragraph rendered no text').not.toBe('');
  for (const text of [standfirst, paragraph]) {
    expect(text).not.toMatch(/nothing (?:ever )?leaves/i);
  }

  const census = providerCensus(modRs, githubRs);
  expect(census.problems).toEqual([]);
  const unauthenticated = census.entries.filter((entry) => entry.auth === 'none');
  console.warn(
    `unauthenticated provider methods: ${String(unauthenticated.length)} (${unauthenticated.map((e) => e.method).join(', ')})`,
  );
  expect(unauthenticated.length, 'the census found no unauthenticated method').toBeGreaterThan(0);
  for (const entry of unauthenticated) {
    const category = required(CENSUS_CATEGORIES[entry.category], `category ${entry.category}`);
    const consent = required(category.consent, `${entry.category}'s consent phrase`);
    const carries = required(category.carries, `${entry.category}'s carries phrase`);
    expect(paragraph).toContain(consent);
    expect(paragraph).toContain(carries);
  }
});

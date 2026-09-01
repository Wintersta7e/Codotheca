import { test, expect } from 'vitest';
import { countGlyph, initialTicks, provenanceLabel, refusedRow, toRow } from './rootRows';
import type { RootSuggestion } from '../../generated/protocol';

const base: RootSuggestion = {
  pathDisplay: '/somewhere/dev',
  kind: 'linux',
  distro: '',
  provenance: 'gitconfig',
  provenanceDetail: 'includeif',
  hits: 3,
  preTicked: true,
};

// §10.1b: the provenance label is the trust argument — a row with no stated source has no
// reason to be ticked — so it decides a tick.
test('every provenance states where the row came from', () => {
  expect(provenanceLabel({ ...base })).toBe('GITCONFIG · includeIf gitdir:/somewhere/dev/');
  expect(
    provenanceLabel({ ...base, provenance: 'editor_recent', provenanceDetail: 'vscode' }),
  ).toBe('VS CODE · RECENT WORKSPACES');
  expect(
    provenanceLabel({ ...base, provenance: 'editor_recent', provenanceDetail: 'jetbrains' }),
  ).toBe('JETBRAINS · RECENT PROJECTS');
  expect(provenanceLabel({ ...base, provenance: 'convention', provenanceDetail: null })).toBe(
    'A COMMON PLACE FOR REPOSITORIES · NO SOURCE NAMED IT',
  );
});

// §10.1a: cloud-sync roots are labelled `cloud-synced — scanning may trigger downloads`,
// uppercased into the row's mono register by §10.1b.
test('the three unticked reasons take the provenance slot', () => {
  expect(provenanceLabel({ ...base, provenance: 'cloud_synced' })).toBe(
    'CLOUD-SYNCED · SCANNING MAY TRIGGER DOWNLOADS',
  );
  expect(provenanceLabel({ ...base, provenance: 'system_root' })).toBe('SYSTEM ROOT');
  expect(provenanceLabel({ ...base, provenance: 'distro', provenanceDetail: 'stopped' })).toBe(
    'NOT RUNNING · SCANNING WILL START IT',
  );
  expect(provenanceLabel({ ...base, provenance: 'distro', provenanceDetail: 'running' })).toBe(
    'A DISTRO ON THIS MACHINE',
  );
});

// §10.1b: n hits read under HITS; `—` where no source named the row; `?` where a count exists
// but is not computed. Never `0`.
test('the count is provenance hits before a walk, and never a fabricated zero', () => {
  expect(countGlyph(3, null)).toEqual({ text: '3', unit: 'HITS' });
  expect(countGlyph(null, null)).toEqual({ text: '—', unit: 'HITS' });
  expect(countGlyph(null, 12)).toEqual({ text: '12', unit: 'PROJECTS' });
  expect(countGlyph(2, 0)).toEqual({ text: '0', unit: 'PROJECTS' });
});

test('a root that has been walked but has no computable count reads as unknown', () => {
  expect(countGlyph(null, -1)).toEqual({ text: '?', unit: 'PROJECTS' });
});

test('pre-ticked rows are ticked and the rest are not', () => {
  const ticks = initialTicks([
    base,
    { ...base, pathDisplay: '/somewhere/synced', provenance: 'cloud_synced', preTicked: false },
  ]);
  expect(ticks.has('/somewhere/dev')).toBe(true);
  expect(ticks.has('/somewhere/synced')).toBe(false);
});

// §10.1b: three refusals are absolute and are listed with no tick slot at all.
test('an absolute refusal is drawn with no tick slot', () => {
  const row = refusedRow(
    { root: null, refusedBecause: 'home_without_narrowing', estimatedDirs: null },
    '/somewhere',
  );
  expect(row.tickable).toBe(false);
  expect(row.ticked).toBe(false);
  expect(row.provenance).toBe('PICK A FOLDER INSIDE YOUR HOME, NOT ALL OF IT');
});

// §10.1b: the fourth is confirmable — over 500k estimated directories keeps its tick.
test('the confirmable refusal keeps its tick', () => {
  const row = refusedRow(
    { root: null, refusedBecause: 'too_many_directories', estimatedDirs: 500_000 },
    '/somewhere/big',
  );
  expect(row.tickable).toBe(true);
  expect(row.ticked).toBe(true);
});

test('a row is keyed by its display string and never by a path the renderer built', () => {
  expect(toRow(base, true).key).toBe(base.pathDisplay);
});

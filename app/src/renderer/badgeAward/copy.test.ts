/**
 * §40.9: every badge kind the schema declares has a name and a sentence, and neither is the
 * slug. The kinds are read from the schema, not from the module, so a kind added there without
 * copy fails here as well as in tsc.
 */
import { describe, expect, it } from 'vitest';

import schemaRaw from '../../../../protocol/schema/protocol.json?raw';
import type { BadgeAward, BadgeKind } from '../../generated/protocol';
import { badgeAwardCopy } from './copy';

function schemaKinds(): string[] {
  expect(schemaRaw.length, 'protocol.json read as an empty string').toBeGreaterThan(0);
  const raw = JSON.parse(schemaRaw) as { types?: Record<string, { variants?: string[] }> };
  return raw.types?.['BadgeKind']?.variants ?? [];
}

function awardOf(kind: string): BadgeAward {
  return {
    kind: kind as BadgeKind,
    earnedOn: '2024-07-03',
    projectId: null,
    evidence: {
      projectName: null,
      gapDays: null,
      year: null,
      languages: null,
      creditedSeconds: null,
      projects: null,
      runStartOn: null,
    },
    landed: false,
  };
}

describe('badgeAwardCopy', () => {
  it('names every schema kind, never by its slug', () => {
    const kinds = schemaKinds();
    console.warn(`BadgeKind variants: ${String(kinds.length)}`, kinds);
    expect(kinds.length, 'the schema declares no BadgeKind').toBeGreaterThan(0);
    for (const kind of kinds) {
      const copy = badgeAwardCopy(awardOf(kind));
      // A kind missing from the table reads as `undefined`, which `not.toBe('')` would pass.
      expect(typeof copy.name, `${kind}: no name`).toBe('string');
      expect(typeof copy.sentence, `${kind}: no sentence`).toBe('string');
      expect(copy.name, `${kind}: name`).not.toBe('');
      expect(copy.sentence, `${kind}: sentence`).not.toBe('');
      expect(copy.name, `${kind}: the name is the slug`).not.toBe(kind);
      expect(copy.sentence, `${kind}: the sentence is the slug`).not.toBe(kind);
    }
  });
});

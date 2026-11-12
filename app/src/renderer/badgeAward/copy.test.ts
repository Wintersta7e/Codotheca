/**
 * §40.9: every badge kind the schema declares has a name and a sentence, and neither is the
 * slug. The kinds are read from the schema, not from the module, so a kind added there without
 * copy fails here as well as in tsc.
 */
import { describe, expect, it } from 'vitest';

import schemaRaw from '../../../../protocol/schema/protocol.json?raw';
import type { BadgeAward, BadgeEvidence, BadgeKind } from '../../generated/protocol';
import { badgeAwardCopy, joinList } from './copy';

function schemaKinds(): string[] {
  expect(schemaRaw.length, 'protocol.json read as an empty string').toBeGreaterThan(0);
  const raw = JSON.parse(schemaRaw) as { types?: Record<string, { variants?: string[] }> };
  return raw.types?.['BadgeKind']?.variants ?? [];
}

const NO_EVIDENCE: BadgeEvidence = {
  projectName: null,
  gapDays: null,
  year: null,
  languages: null,
  creditedSeconds: null,
  projects: null,
  runStartOn: null,
};

function awardOf(kind: string, evidence: Partial<BadgeEvidence> = {}): BadgeAward {
  return {
    kind: kind as BadgeKind,
    earnedOn: '2024-07-03',
    projectId: null,
    evidence: { ...NO_EVIDENCE, ...evidence },
    landed: false,
  };
}

const sentence = (kind: BadgeKind, evidence: Partial<BadgeEvidence> = {}): string =>
  badgeAwardCopy(awardOf(kind, evidence)).sentence;

const projects = (...names: string[]): BadgeEvidence['projects'] =>
  names.map((name) => ({ name, evaluable: null }));

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

  // §40.9: names are copy and the slug is never rendered — in any case, in any kind's words.
  it('gives each kind its own upper-case name, and no slug appears in any name or sentence', () => {
    const kinds = schemaKinds();
    const copies = kinds.map((kind) => badgeAwardCopy(awardOf(kind)));
    expect(new Set(copies.map((copy) => copy.name)).size).toBe(kinds.length);
    for (const [index, copy] of copies.entries()) {
      expect(copy.name, kinds[index]).toBe(copy.name.toUpperCase());
      for (const slug of kinds) {
        expect(`${copy.name} ${copy.sentence}`.toLowerCase(), kinds[index]).not.toContain(slug);
      }
    }
  });

  // §40.9: every evidence field is nullable; a missing one drops its clause, never prints `0`,
  // `null` or an empty name, and the only figure left is the date the award was earned.
  it('reads whole with no evidence, and states no figure but the date', () => {
    for (const kind of schemaKinds()) {
      const text = sentence(kind as BadgeKind);
      console.warn(`${kind}: ${text}`);
      expect(text, kind).not.toMatch(/‹COPY|null|undefined|NaN/u);
      expect(text.replaceAll('2024-07-03', ''), kind).not.toMatch(/\d/u);
      expect(text, kind).not.toMatch(/\s[,.]|\s{2}|^\s|'s's/u);
      expect(text, kind).toMatch(/^[A-Z].*\.$/u);
    }
  });

  it("states each kind's evidence in its own sentence", () => {
    expect(sentence('necromancer', { projectName: 'alpha', gapDays: 200 })).toBe(
      'alpha had a commit again on 2024-07-03, after 200 days dormant.',
    );
    expect(sentence('necromancer')).toBe('A project had a commit again on 2024-07-03.');
    expect(sentence('polyglot', { year: 2024, languages: ['C', 'Go', 'Rust', 'TS', 'Zig'] })).toBe(
      'In 2024 you committed to projects whose main languages were C, Go, Rust, TS and Zig.',
    );
    expect(sentence('polyglot')).toBe(
      'In one year you committed to projects in several main languages.',
    );
    expect(sentence('comeback', { gapDays: 45 })).toBe(
      'Welcome back. Work picked up again on 2024-07-03, 45 days after the commit-day before it.',
    );
    expect(sentence('comeback')).toBe('Welcome back. Work picked up again on 2024-07-03.');
    expect(sentence('curator', { projects: projects('alpha', 'beta') })).toBe(
      'alpha and beta each passed every check that could be scored, as observed on 2024-07-03.',
    );
    expect(sentence('curator')).toBe(
      'Your projects each passed every check that could be scored, as observed on 2024-07-03.',
    );
    // PLAYTIME's one formatter: 15,120 credited seconds is 4.2h, the session's own figure.
    expect(sentence('marathon', { projectName: 'alpha', creditedSeconds: 15_120 })).toBe(
      'One session of alpha counted 4.2h of playtime, ending 2024-07-03.',
    );
    expect(sentence('marathon')).toBe('One long session of a project, ending 2024-07-03.');
    expect(sentence('undertaker', { projects: projects('a', 'b', 'c', 'd', 'e') })).toBe(
      'a, b, c, d and e were each archived cleanly, as last observed.',
    );
    expect(sentence('undertaker')).toBe(
      'Projects on your shelf were each archived cleanly, as last observed.',
    );
    expect(sentence('firefighter', { projectName: 'alpha' })).toBe(
      "On 2024-07-03, alpha's build went red and turned green the same day, a day you committed there.",
    );
    expect(sentence('firefighter')).toBe(
      "On 2024-07-03, a project's build went red and turned green the same day, a day you committed there.",
    );
    expect(sentence('lifeline', { runStartOn: '2024-06-04' })).toBe(
      "From 2024-06-04 to 2024-07-03, each project's checked-out branch was pushed as last observed, every day.",
    );
    expect(sentence('lifeline')).toBe(
      "Up to 2024-07-03, each project's checked-out branch was pushed as last observed, every day.",
    );
  });

  // An empty name is no name: it falls back as a null does.
  it('treats an empty name or an empty list as absent', () => {
    expect(sentence('necromancer', { projectName: '' })).toBe(sentence('necromancer'));
    expect(sentence('polyglot', { languages: [] })).toBe(sentence('polyglot'));
    expect(sentence('curator', { projects: projects('') })).toBe(sentence('curator'));
    expect(sentence('undertaker', { projects: [] })).toBe(sentence('undertaker'));
    expect(sentence('lifeline', { runStartOn: '' })).toBe(sentence('lifeline'));
  });

  // Curator's list has no upper bound and the band line is one button: three names, then how
  // many more. Undertaker's evidence holds its five, and all five are named.
  it('lists at most three Curator projects, then how many more', () => {
    const seven = projects('a', 'b', 'c', 'd', 'e', 'f', 'g');
    expect(sentence('curator', { projects: seven })).toMatch(/^a, b, c and 4 more each passed /u);
    expect(sentence('curator', { projects: projects('a', 'b', 'c', 'd') })).toMatch(
      /^a, b, c and 1 more each /u,
    );
    expect(sentence('curator', { projects: projects('a', 'b', 'c') })).toMatch(
      /^a, b and c each /u,
    );
  });
});

describe('joinList', () => {
  it('joins with commas and one final "and"', () => {
    expect(joinList([])).toBe('');
    expect(joinList(['a'])).toBe('a');
    expect(joinList(['a', 'b'])).toBe('a and b');
    expect(joinList(['a', 'b', 'c', 'd', 'e'])).toBe('a, b, c, d and e');
  });
});

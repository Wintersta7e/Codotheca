/**
 * Every variant the schema generates has a readable label, so a tenth source cannot ship showing
 * its raw id. The variant sets are read from the **tracked** schema, which both languages are
 * generated from, never from `src/generated` (gitignored).
 */
import { describe, expect, it } from 'vitest';
// `?raw` rather than node:fs: the renderer project carries no Node types by design.
import schemaRaw from '../../../../../protocol/schema/protocol.json?raw';
import type { CompletionCheck, DebtSource, DecayLayer } from '../../../generated/protocol';
import { CHECK_LABELS } from '../completion/checklist';
import {
  checkLabel,
  itemLabel,
  LAYER_LABELS,
  layerLabel,
  SOURCE_LABELS,
  type SourceLabel,
} from './labels';

function variantsOf(type: string): string[] {
  expect(schemaRaw.length, 'protocol.json read as an empty string').toBeGreaterThan(0);
  const raw = JSON.parse(schemaRaw) as { types?: Record<string, { variants?: string[] }> };
  return raw.types?.[type]?.variants ?? [];
}

/** A label that is the raw id is the defect this table exists to end. */
function expectWords(label: string | undefined, variant: string): void {
  expect(label ?? '', `no label for ${variant}`).not.toBe('');
  expect(label, variant).not.toBe(variant);
  expect(label, variant).not.toMatch(/_/u);
}

describe('readable labels, one owner', () => {
  it('names every generated DebtSource variant twice: as a check and as an item', () => {
    const variants = variantsOf('DebtSource');
    console.warn(`labels: DebtSource variants read from the schema: ${String(variants.length)}`);
    expect(variants.length, 'the schema declares no DebtSource variant').toBeGreaterThan(0);
    const table = SOURCE_LABELS as Readonly<Partial<Record<string, SourceLabel>>>;
    for (const variant of variants) {
      expectWords(table[variant]?.check, `${variant} as a check`);
      expectWords(table[variant]?.item, `${variant} as an item`);
      // One name for both would put the problem's words beside PASSED again.
      expect(table[variant]?.check, variant).not.toBe(table[variant]?.item);
    }
    // And no label for a variant the schema no longer declares.
    expect(Object.keys(table).sort()).toEqual([...variants].sort());
  });

  it('labels every generated DecayLayer variant', () => {
    const variants = variantsOf('DecayLayer');
    console.warn(`labels: DecayLayer variants read from the schema: ${String(variants.length)}`);
    expect(variants.length, 'the schema declares no DecayLayer variant').toBeGreaterThan(0);
    const table = LAYER_LABELS as Readonly<Partial<Record<string, string>>>;
    for (const variant of variants) expectWords(table[variant], variant);
    expect(Object.keys(table).sort()).toEqual([...variants].sort());
  });

  it('names a check for what it looks at, never as a statement of the problem', () => {
    // A check row reads `<name> · PASSED` and a switch `<name>, switch, on`: a name that states
    // the problem (`No README`, `CI is red`) is false beside every passing project.
    for (const [source, { check }] of Object.entries(SOURCE_LABELS)) {
      expect(check, source).not.toMatch(/^(no|not)\b|\b(is|are)\b/iu);
    }
  });

  it('names a check as the checklist on the same tab names its pair', () => {
    // §28.2's six one-to-one pairs: closing the item and lighting the tick are one event, so the
    // tab calls the one fact by one name.
    const pairs: readonly (readonly [DebtSource, CompletionCheck])[] = [
      ['missing_readme', 'readme'],
      ['missing_license', 'license'],
      ['missing_tests', 'tests'],
      ['unpushed_commits', 'pushed'],
      ['ci_red', 'ciGreen'],
      ['no_release', 'release'],
    ];
    for (const [source, check] of pairs) {
      expect(SOURCE_LABELS[source].check.toUpperCase(), source).toBe(CHECK_LABELS[check]);
    }
  });

  it('names a variant this build does not know by its id, never blank', () => {
    // A newer core can send a tenth source or a sixth layer. `toString` is on every object's
    // prototype, so a plain lookup would hand back a function rather than nothing.
    for (const id of ['tenth_source', 'toString']) {
      expect(checkLabel(id as DebtSource)).toBe(id);
      expect(itemLabel(id as DebtSource)).toBe(id);
    }
    expect(layerLabel('sediment' as DecayLayer)).toBe('SEDIMENT');
    expect(layerLabel('toString' as DecayLayer)).toBe('TOSTRING');
  });

  it('says none of the words the feature never says', () => {
    const labels = [
      ...Object.values(SOURCE_LABELS).flatMap(({ check, item }) => [check, item]),
      ...Object.values(LAYER_LABELS),
    ];
    for (const label of labels) {
      for (const banned of ['clean', 'healthy', 'none', 'all']) {
        expect(label).not.toMatch(new RegExp(`\\b${banned}\\b`, 'iu'));
      }
    }
  });
});

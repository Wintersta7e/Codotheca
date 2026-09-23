/**
 * Every variant the schema generates has a readable label, so a tenth source cannot ship showing
 * its raw id. The variant sets are read from the **tracked** schema, which both languages are
 * generated from, never from `src/generated` (gitignored).
 */
import { describe, expect, it } from 'vitest';
// `?raw` rather than node:fs: the renderer project carries no Node types by design.
import schemaRaw from '../../../../../protocol/schema/protocol.json?raw';
import { LAYER_LABELS, SOURCE_LABELS } from './labels';

function variantsOf(type: string): string[] {
  expect(schemaRaw.length, 'protocol.json read as an empty string').toBeGreaterThan(0);
  const raw = JSON.parse(schemaRaw) as { types?: Record<string, { variants?: string[] }> };
  return raw.types?.[type]?.variants ?? [];
}

describe('readable labels, one owner', () => {
  for (const [type, labels] of [
    ['DebtSource', SOURCE_LABELS],
    ['DecayLayer', LAYER_LABELS],
  ] as const) {
    it(`labels every generated ${type} variant`, () => {
      const variants = variantsOf(type);
      console.warn(`labels: ${type} variants read from the schema: ${String(variants.length)}`);
      expect(variants.length, `the schema declares no ${type} variant`).toBeGreaterThan(0);
      const table = labels as Readonly<Record<string, string>>;
      for (const variant of variants) {
        const label = table[variant] ?? '';
        expect(label, `no label for ${variant}`).not.toBe('');
        // A label that is the raw id is the defect this table exists to end.
        expect(label, variant).not.toBe(variant);
        expect(label, variant).not.toMatch(/_/u);
      }
      // And no label for a variant the schema no longer declares.
      expect(Object.keys(table).sort()).toEqual([...variants].sort());
    });
  }

  it('says none of the words the feature never says', () => {
    for (const label of [...Object.values(SOURCE_LABELS), ...Object.values(LAYER_LABELS)]) {
      for (const banned of ['clean', 'healthy', 'none', 'all']) {
        expect(label).not.toMatch(new RegExp(`\\b${banned}\\b`, 'iu'));
      }
    }
  });
});

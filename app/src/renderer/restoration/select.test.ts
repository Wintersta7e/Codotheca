import { describe, expect, it } from 'vitest';
import schemaRaw from '../../../../protocol/schema/protocol.json?raw';
import type { DecayLayer, HealthLayerDelta } from '../../generated/protocol';
import { DECAY_LAYER_ORDER } from '../decay/layers';
import { selectOrigin, type Selection } from './select';
import selectSource from './select.ts?raw';

/** `DecayLayer`'s variants as the schema declares them, in declaration order. */
function schemaOrder(): readonly string[] {
  expect(schemaRaw.length, 'protocol.json read as an empty string').toBeGreaterThan(0);
  const schema = JSON.parse(schemaRaw) as { types: Record<string, { variants?: string[] }> };
  const variants = schema.types['DecayLayer']?.variants ?? [];
  expect(variants.length, 'the schema declares no DecayLayer variant').toBeGreaterThan(0);
  return variants;
}

function delta(layer: DecayLayer, from: number | null, to: number | null): HealthLayerDelta {
  return { layer, fromValue: from, toValue: to };
}

const NOTHING_ELSE: ReadonlyMap<DecayLayer, number> = new Map();

/**
 * **`AC-P3-34-5`** — A3's selection: the largest **signed** decrease, never the largest delta.
 *
 * D6 §1's `|to − from|` rule is superseded because it picks a WORSENING layer, and a layer's value
 * is an open-item count, so a worsening layer is an ordinary event rather than a corner case.
 */
describe('ac p3 34 5 — the surge originates at the largest decrease', () => {
  it('ac_p3_34_5 selects by signed decrease, the declared order, and the two no-origin cases', () => {
    const order = schemaOrder();
    // The tie-break reads the generated enum's order, so the expectation is derived from the
    // schema rather than written here: a reordering moves both together.
    const [first, second] = [order[0], order[1]] as [DecayLayer, DecayLayer];

    const cases: readonly {
      name: string;
      layers: readonly HealthLayerDelta[];
      elsewhere: ReadonlyMap<DecayLayer, number>;
      expected: Selection;
    }[] = [
      {
        name: 'rust improving beside dust worsening selects rust, not dust',
        layers: [delta('rust', 2, 1), delta('dust', 1, 11)],
        elsewhere: NOTHING_ELSE,
        expected: { kind: 'layer', layer: 'rust' },
      },
      {
        name: 'two equal decreases select the earlier declared variant',
        layers: [delta(second, 3, 1), delta(first, 5, 3)],
        elsewhere: new Map([['overgrowth', 4]]),
        expected: { kind: 'layer', layer: first },
      },
      {
        name: 'an event in which nothing decreased selects nothing',
        layers: [delta('dust', 1, 3), delta('cracks', 2, 2)],
        elsewhere: NOTHING_ELSE,
        expected: { kind: 'none' },
      },
      {
        name: 'every lit layer reaching zero is the whole card, no single cause',
        layers: [delta('dust', 2, 0), delta('rust', 5, 0)],
        elsewhere: NOTHING_ELSE,
        expected: { kind: 'whole' },
      },
      {
        name: 'a still-lit layer outside the event keeps it from being the whole card',
        layers: [delta('dust', 2, 0), delta('rust', 5, 0)],
        elsewhere: new Map([['cracks', 1]]),
        expected: { kind: 'layer', layer: 'rust' },
      },
      {
        name: 'a NULL fromValue never selects — unknown is never rendered as zero',
        layers: [delta('overgrowth', null, 0)],
        elsewhere: NOTHING_ELSE,
        expected: { kind: 'none' },
      },
      {
        name: 'a NULL toValue never selects either',
        layers: [delta('cracks', 4, null), delta('dust', 3, 2)],
        elsewhere: NOTHING_ELSE,
        expected: { kind: 'layer', layer: 'dust' },
      },
    ];

    let executed = 0;
    for (const c of cases) {
      expect(selectOrigin(c.layers, c.elsewhere), c.name).toEqual(c.expected);
      executed += 1;
    }
    console.error(`AC-P3-34-5 selection cases executed: ${String(executed)}`);
    expect(executed, 'a table that executed nothing proved nothing').toBeGreaterThan(0);
  });

  it('ac_p3_34_5 is pure: same input, same selection, and no clock, storage or React', () => {
    const layers = [delta('rust', 2, 1), delta('cracks', 3, 1)];
    expect(selectOrigin(layers, NOTHING_ELSE)).toEqual(selectOrigin(layers, NOTHING_ELSE));

    const source = selectSource;
    expect(source.length, 'select.ts read as an empty string').toBeGreaterThan(0);
    const forbidden: readonly [string, RegExp][] = [
      ['a clock', /\bDate\b|\bperformance\.now\b/u],
      ['storage', /\blocalStorage\b|\bsessionStorage\b|\bindexedDB\b/u],
      ['React', /from\s+'react'/u],
    ];
    for (const [what, pattern] of forbidden) {
      expect(pattern.test(source), `select.ts reaches for ${what}`).toBe(false);
    }
  });

  it('ac_p3_34_5 ties break on the order the schema declares, read from the schema', () => {
    expect([...DECAY_LAYER_ORDER]).toEqual(schemaOrder());
  });
});

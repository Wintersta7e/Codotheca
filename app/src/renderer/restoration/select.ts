/**
 * §34.5's selection (A3): **which layer a restoration originates at**, or that none does.
 *
 * D6 §1's largest-`|to − from|` rule is superseded. The absolute bars select a **worsening**
 * layer — `rust` improving 2 → 1 beside `dust` worsening 1 → 11 picks `dust` — and §33 defines a
 * layer's value as an open-item count, so a worsening layer is an ordinary event, not a corner
 * case. Only a decrease originates anything.
 *
 * **Pure**: the same input gives the same selection, with no clock and no stored state. A row
 * whose `fromValue` or `toValue` is NULL never animates — *never render unknown as zero*, applied
 * to motion.
 */
import type { DecayLayer, HealthLayerDelta } from '../../generated/protocol';
import { DECAY_LAYER_ORDER } from '../decay/layers';

export type Selection =
  | { readonly kind: 'none' }
  | { readonly kind: 'whole' }
  | { readonly kind: 'layer'; readonly layer: DecayLayer };

/**
 * §34.5's five steps, in order.
 *
 * `litElsewhere` is the page's lit count per layer as it stood when the event arrived. **It is
 * here because the event carries only the layers that CHANGED**: step 5's *every layer holding a
 * nonzero value reaches 0* cannot be decided from the event alone, and deciding it from the event
 * would light the whole card while a layer the event does not mention is still lit — the canned
 * full-clean rendering in exactly the case §34.5 calls it a lie. An unchanged layer has the same
 * value before and after the event, so the page's current count is the right one for it.
 */
export function selectOrigin(
  layers: readonly HealthLayerDelta[],
  litElsewhere: ReadonlyMap<DecayLayer, number>,
): Selection {
  // Steps 1 and 4: only an observed decrease counts, and no decrease is no restoration.
  let best: { layer: DecayLayer; drop: number } | null = null;
  for (const entry of layers) {
    if (entry.fromValue === null || entry.toValue === null) continue;
    const drop = entry.fromValue - entry.toValue;
    if (drop <= 0) continue;
    // Steps 2 and 3: the largest signed decrease, ties on the declared order, which A14.1 makes
    // total for exactly this purpose.
    if (
      best === null ||
      drop > best.drop ||
      (drop === best.drop &&
        DECAY_LAYER_ORDER.indexOf(entry.layer) < DECAY_LAYER_ORDER.indexOf(best.layer))
    ) {
      best = { layer: entry.layer, drop };
    }
  }
  if (best === null) return { kind: 'none' };

  // Step 5, consulted before steps 1–3's answer: every layer ends at zero, so there is no single
  // cause and the honest rendering is the whole card.
  const cleared = DECAY_LAYER_ORDER.every((layer) => {
    const entry = layers.find((e) => e.layer === layer);
    if (entry !== undefined) return entry.toValue === 0;
    return (litElsewhere.get(layer) ?? 0) === 0;
  });
  if (cleared) return { kind: 'whole' };

  return { kind: 'layer', layer: best.layer };
}

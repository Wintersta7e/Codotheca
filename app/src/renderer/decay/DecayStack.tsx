/**
 * §33.4 — the five material layers, composed **in the DOM over a bitmap decay never enters**.
 *
 * **One class, not five.** Every layer element carries `cdt-decay` and distinguishes itself by
 * `data-decay-layer`: five names in the tier clamp would be five chances to miss one.
 *
 * **The element is a function of the sprite count, never of the lit bit.** A layer with zero lit
 * anchors renders **no** `.cdt-decay` element at all. A node that claims a layer and draws
 * nothing is exactly what invites a later *"a lit layer has an element"* assumption, which is
 * green on `Screen` and red on `Plain` — the majority shape. **The layer set is never the only
 * rendering of an open item**: §30's page lists every one of them in text, unconditionally, so an
 * invisible layer loses ornament and no fact.
 *
 * **Never on the grid, in Peek, in the list, in the palette, in triage or on the Amnesty card** —
 * the same scope roasting has. The mechanism is a prop: `CardPlate` renders whatever `decay` it
 * is handed, and only the opened hero hands it one. A surface that forgets to pass it renders
 * nothing, which is the correct default. **This is also the performance answer**: five extra
 * elements on the one card on screen, never five on each of 140 mounted tiles.
 */
import type { CSSProperties, ReactElement } from 'react';
import type { DebtItem, DecayLayer, Weathering } from '../../generated/protocol';
import { DECAY_LAYER_ORDER, anchorPercents, type AnchorBox } from './layers';
import { litAnchorCount, litCounts } from './lit';

export interface DecayStackProps {
  /** The core's resolved anchor set for the scene on screen. `null` mounts nothing. */
  readonly weathering: Weathering | null;
  /** §28's flat item list, straight off `ProjectDetail`. */
  readonly debt: readonly DebtItem[];
}

function spriteKey(name: DecayLayer, index: number): string {
  // The anchor list IS the order the layer lights in, and nothing reorders it: the index is the
  // sprite's identity.
  return `${name}-${String(index)}`;
}

function spriteStyle(box: AnchorBox): CSSProperties {
  return {
    left: `${String(box.leftPct)}%`,
    top: `${String(box.topPct)}%`,
    width: `${String(box.widthPct)}%`,
    height: `${String(box.heightPct)}%`,
  };
}

export function DecayStack(props: DecayStackProps): ReactElement | null {
  const { weathering } = props;
  if (weathering === null) return null;

  const counts = litCounts(props.debt);
  const elements: ReactElement[] = [];

  for (const name of DECAY_LAYER_ORDER) {
    const entry = weathering.layers.find((l) => l.layer === name);
    if (entry === undefined) continue;
    const boxes = anchorPercents(entry, weathering.spaceW, weathering.spaceH);
    // The first `min(value, anchors)` anchors, in §33.3's declared order. Saturation is
    // `litAnchorCount`'s and is expressed in one place.
    const lit = litAnchorCount(counts.get(name) ?? 0, boxes.length);
    if (lit === 0) continue;
    elements.push(renderLayer(name, boxes.slice(0, lit)));
  }

  if (elements.length === 0) return null;
  return <>{elements}</>;
}

function renderLayer(name: DecayLayer, boxes: readonly AnchorBox[]): ReactElement {
  return (
    <span className="cdt-decay" data-decay-layer={name} aria-hidden="true" key={name}>
      {boxes.map((box, index) => (
        <span className="cdt-decay-sprite" key={spriteKey(name, index)} style={spriteStyle(box)} />
      ))}
    </span>
  );
}

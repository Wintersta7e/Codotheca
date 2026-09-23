/**
 * §11.6's `Restoration surge` beat — **the one duration the renderer reads for the surge.**
 *
 * §11.6 owns it, as it owns every other beat and curve; §34.7 states the requirement and this is
 * its only reader in code. The stylesheet declares the same figure on `.cdt-surge`, and
 * `css-vocabulary.json` is what lets `check-style-tokens.mjs` accept it — `envelope.test.ts`
 * reads that file, so the two cannot drift apart silently.
 *
 * Several layers closing together arrive under this one envelope: never a second one, and never
 * queued back to back.
 */
export const RESTORATION_SURGE_MS = 2300;

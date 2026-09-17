/**
 * Types for `markup-chunk.mjs`, so the renderer's Vite config and the post-build gate can read
 * **one** declaration of §25.5's lazy chunk rather than each carrying its own copy.
 *
 * The config decides what goes into the chunk; the gate decides what first paint may not load.
 * Two copies of that set is R12's shape on the gate this lane rewrote because its predecessor
 * was inert.
 */
export declare const README_MARKUP_CHUNK: string;
export declare const MARKUP_PACKAGES: readonly string[];

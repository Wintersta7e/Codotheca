/**
 * §25.5's lazy chunk, named **once**.
 *
 * Two consumers decide two halves of one guarantee and they must be talking about the same set:
 * `app/electron.vite.config.ts` decides which packages go into the chunk, and
 * `scripts/check-bundle.mjs` decides which packages first paint may not load. When each carried
 * its own copy, adding a fifth library to the config left the gate checking four — and a
 * statically imported fifth would have ridden into `readme-markup`, been pulled in eagerly, and
 * passed.
 *
 * It is its own module rather than an export of the gate, because `check-bundle.mjs` runs its
 * checks at import time and calls `process.exit` on failure: importing it from a Vite config
 * would run the gate whenever the config loaded.
 */

/** The chunk the panel's one dynamic `import()` pulls. */
export const README_MARKUP_CHUNK = 'readme-markup';

/** The four libraries §25.5 lazy-loads, and the only ones this chunk exists for. */
export const MARKUP_PACKAGES = ['markdown-it', 'dompurify', 'highlight.js', 'katex'];

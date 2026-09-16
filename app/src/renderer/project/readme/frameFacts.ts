/**
 * The two §25.5 values the **panel** needs and the frame builder also names, in a module of their
 * own so neither side restates the other.
 *
 * They are here rather than in `frame.ts` because the panel must reach them **without** importing
 * the markup stack: the one dynamic `import()` that keeps `markdown-it`, DOMPurify,
 * `highlight.js` and KaTeX off the first-paint chunk would be defeated by a static import of a
 * constant that happens to live beside them. `frame.ts` re-exports both, so there is one owner
 * either way you arrive.
 */

/** A scriptless frame cannot report its own height, so the panel gives it one and it scrolls. */
export const FRAME_HEIGHT_PX = 420;

/**
 * §25.5's statement, rendered **once** below the frame and only where the document has an anchor.
 * Never as standing furniture on a document that has no links.
 */
export const LINKS_INERT_NOTICE = 'LINKS ARE NOT ACTIVE IN THIS PANEL';

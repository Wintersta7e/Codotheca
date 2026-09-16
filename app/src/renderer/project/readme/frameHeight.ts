/**
 * §25.5's frame height, alone in a module of its own.
 *
 * A scriptless frame cannot report its own height, so the panel gives it one — and the panel must
 * know the number **without** importing the markup stack, or the one dynamic `import()` that keeps
 * `markdown-it`, DOMPurify, `highlight.js` and KaTeX off the first-paint chunk would be defeated
 * by a static import of a constant. `frame.ts` re-exports this, so there is still one owner.
 */
export const FRAME_HEIGHT_PX = 420;

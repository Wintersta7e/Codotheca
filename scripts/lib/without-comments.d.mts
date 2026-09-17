/**
 * Types for `without-comments.mjs`, so a gate written in TypeScript can strip comments the way
 * the `scripts/` gates do rather than growing a second regex pair that drifts from this one.
 *
 * Comments are replaced with spaces, never removed: line numbers and offsets are preserved, so a
 * match's position in the stripped text is its position in the file.
 */
export declare function withoutComments(source: string): string;

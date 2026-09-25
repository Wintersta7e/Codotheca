/**
 * A handler with nothing to do: an ignored callback, or a test's stand-in for one it does not
 * observe. Named so a deliberate no-op reads as one, where an inline `() => {}` reads as a body
 * someone forgot to write.
 */
export function noop(): void {
  // Nothing to do, by definition.
}

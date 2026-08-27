/**
 * §3.1's git version floor as `[major, minor]`, mirroring `GIT_FLOOR` in
 * `core/src/git/version.rs`. The core enforces it; §11.2a's git-floor failure window prints it,
 * and the protocol cannot carry a compile-time constant. `gitFloor.test.ts` reads the Rust
 * source and asserts the two are equal — R24's condition on a cross-language mirror, and the
 * only thing keeping this file from stating a floor the core does not enforce.
 */
export const GIT_FLOOR: readonly [number, number] = [2, 22];

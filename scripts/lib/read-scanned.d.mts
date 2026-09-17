/**
 * Types for `read-scanned.mjs`, so a gate written in TypeScript can use the same helper the
 * `scripts/` gates do rather than growing its own `readFileSync` and the ENOENT crash that
 * helper exists to prevent.
 *
 * `null` means the file was gone by the time it was read. A caller must skip it **before**
 * counting it, or its "scanned nothing" guard stops meaning what it says.
 */
export declare function readScannedFile(path: string): string | null;

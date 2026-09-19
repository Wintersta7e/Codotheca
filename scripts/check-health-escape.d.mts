/**
 * Types for `check-health-escape.mjs`, so a TypeScript test can drive the same gate rather than
 * growing a second copy of its rules.
 *
 * `healthIdentifiers` is the one that crosses: `app/src/renderer/shelf/counts.test.ts` asserts
 * that none of them reaches the module producing the attention-row counts, and deriving the set
 * a second time there would be the defect the derivation exists to prevent.
 */
export interface HealthEscapeHit {
  readonly path: string;
  readonly line: number;
  readonly token: string;
}

export interface HealthEscapeViolation {
  readonly path: string;
  readonly escapes: readonly { readonly line: number; readonly token: string }[];
  readonly identifiers: readonly { readonly line: number; readonly token: string }[];
}

export declare const SCAN_ROOTS: readonly string[];
export declare const ESCAPE_CALLS: readonly string[];

/** The banned set, derived from `protocol/schema/protocol.json`. Never a literal list. */
export declare function healthIdentifiers(schema: unknown): string[];

export declare function collectFiles(roots: readonly string[], repoRoot: string): string[];

export declare function scanSource(
  source: string,
  path: string,
  identifiers: readonly string[],
): { escapes: HealthEscapeHit[]; violations: HealthEscapeViolation[] };

export declare function scanFiles(
  files: readonly string[],
  repoRoot: string,
  identifiers: readonly string[],
): { scanned: number; escapeSites: HealthEscapeHit[]; violations: HealthEscapeViolation[] };

export declare function main(argv: readonly string[], base?: string): number;

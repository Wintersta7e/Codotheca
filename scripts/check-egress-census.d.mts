/**
 * Types for `check-egress-census.mjs`, so a TypeScript test can read the same census rather than
 * growing a second copy of its rules.
 *
 * `providerCensus` and `CENSUS_CATEGORIES` are the ones that cross: the first-run consent test
 * names every provider method that sends no credential by the phrases this table gives, so a new
 * unauthenticated destination fails there until the consent copy names it.
 */
export interface CensusCategory {
  readonly name: string;
  readonly readme: string;
  readonly security: string;
  readonly consent?: string;
  readonly carries?: string;
}

export declare const MARK_START: string;
export declare const MARK_END: string;
export declare const CENSUS_CATEGORIES: Readonly<Record<string, CensusCategory>>;

export interface ProviderEntry {
  readonly method: string;
  readonly category: string;
  readonly auth: 'token' | 'none';
}

export declare function providerCensus(
  modRs: string,
  githubRs: string,
): { entries: ProviderEntry[]; problems: string[] };

export declare function intentCensus(intentRs: string): {
  entries: { variant: string; network: boolean; category: string | null }[];
  problems: string[];
};

export declare function httpSiteCensus(files: readonly { path: string; text: string }[]): {
  entries: { file: string; sites: number; category: string | null; detail: string }[];
  problems: string[];
};

export declare function cspCensus(cspTs: string): {
  tokens: number;
  origins: string[];
  problems: string[];
};

export declare function shellLoadCensus(files: readonly { path: string; text: string }[]): {
  entries: {
    key: string;
    sites: number;
    network: boolean;
    category: string | null;
    detail: string;
  }[];
  problems: string[];
};

export declare function deriveCensus(root: string): {
  entries: { source: string; category: string | null; detail: string }[];
  perSource: Record<string, number>;
  scanned: number;
  categories: string[];
  problems: string[];
};

export declare function mirrorProblems(
  readme: string,
  security: string,
  categories: readonly string[],
): string[];

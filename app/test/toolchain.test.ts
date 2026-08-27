import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { join } from 'node:path';

const appDir = fileURLToPath(new URL('..', import.meta.url));

function readTsconfig(name: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(readFileSync(join(appDir, name), 'utf8'));
  if (typeof parsed !== 'object' || parsed === null) {
    throw new Error(`${name} is not a JSON object`);
  }
  return parsed as Record<string, unknown>;
}

function compilerOptions(name: string): Record<string, unknown> {
  const options = readTsconfig(name)['compilerOptions'];
  if (typeof options !== 'object' || options === null) {
    throw new Error(`${name} declares no compilerOptions`);
  }
  return options as Record<string, unknown>;
}

// The exact list in 00-index.md's TypeScript row. Losing one is the failure this test exists
// to catch: a bundler migration that "still typechecks" because a flag quietly went away.
const REQUIRED_STRICT_FLAGS = [
  'strict',
  'noUncheckedIndexedAccess',
  'exactOptionalPropertyTypes',
  'noImplicitOverride',
  'noImplicitReturns',
  'noFallthroughCasesInSwitch',
  'noPropertyAccessFromIndexSignature',
  'useUnknownInCatchVariables',
] as const;

describe('the app TypeScript projects', () => {
  it('sets every required strict flag in the shared base', () => {
    const options = compilerOptions('tsconfig.base.json');
    for (const flag of REQUIRED_STRICT_FLAGS) {
      expect(options[flag], `tsconfig.base.json must set ${flag}`).toBe(true);
    }
  });

  it('has both real projects inherit that base', () => {
    for (const name of ['tsconfig.node.json', 'tsconfig.web.json']) {
      expect(readTsconfig(name)['extends']).toBe('./tsconfig.base.json');
    }
  });

  it('keeps Node types out of the renderer project', () => {
    // The renderer is sandboxed and may never originate a filesystem path or an executable.
    // Withholding @types/node makes that a compile error rather than a review comment.
    const types = compilerOptions('tsconfig.web.json')['types'];
    expect(Array.isArray(types)).toBe(true);
    expect(types).not.toContain('node');
  });

  it('leaves shared tests to the node project, which is the only one that can type them', () => {
    // src/shared is imported by both sides, so both projects include it — but a shared *test*
    // runs under Node and may name a Node API: gitFloor.test.ts reads the Rust source it
    // mirrors. Type-checking those here fails on `node:fs` purely because this project
    // withholds @types/node, which is the guarantee above. tsconfig.node.json still covers them,
    // so nothing goes unchecked.
    expect(readTsconfig('tsconfig.web.json')['exclude']).toContain('src/shared/**/*.test.ts');
    expect(readTsconfig('tsconfig.node.json')['include']).toContain('src/shared/**/*.ts');
  });
});

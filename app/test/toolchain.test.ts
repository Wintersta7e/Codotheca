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

// Landed with the strict lint set, each measured clean across all three projects. Two are
// `false` because the strict setting of those flags is the off one.
const REQUIRED_STRICT_VALUES = {
  verbatimModuleSyntax: true,
  erasableSyntaxOnly: true,
  noUncheckedSideEffectImports: true,
  allowUnreachableCode: false,
  allowUnusedLabels: false,
} as const;

describe('the app TypeScript projects', () => {
  it('sets every required strict flag in the shared base', () => {
    const options = compilerOptions('tsconfig.base.json');
    for (const flag of REQUIRED_STRICT_FLAGS) {
      expect(options[flag], `tsconfig.base.json must set ${flag}`).toBe(true);
    }
    for (const [flag, value] of Object.entries(REQUIRED_STRICT_VALUES)) {
      expect(options[flag], `tsconfig.base.json must set ${flag} to ${String(value)}`).toBe(value);
    }
  });

  it('has every project inherit that base', () => {
    for (const name of ['tsconfig.node.json', 'tsconfig.web.json', 'tsconfig.e2e.json']) {
      expect(readTsconfig(name)['extends']).toBe('./tsconfig.base.json');
    }
  });

  it('gives the e2e specs their own project, and typechecks it', () => {
    // A Playwright spec is neither of the other two: it runs under Node *and* it drives a
    // browser, and one that measures a real component's layout imports the renderer's TSX.
    // Under the node project that is `--jsx is not set` plus a missing DOM lib; under the web
    // project it is a missing @types/node. Leaving it in the node project would mean either
    // weakening the guarantee below for main and preload code, or not checking the spec at all.
    const options = compilerOptions('tsconfig.e2e.json');
    expect(options['jsx']).toBe('react-jsx');
    expect(options['types']).toContain('node');
    expect(options['lib']).toContain('DOM');
    expect(readTsconfig('tsconfig.e2e.json')['include']).toContain('e2e/**/*.ts');
    // And the node project must not still carry them, or the stricter project checks the same
    // file and fails on exactly what the new one exists to allow.
    expect(readTsconfig('tsconfig.node.json')['include']).not.toContain('e2e/**/*.ts');

    const pkg: unknown = JSON.parse(readFileSync(join(appDir, 'package.json'), 'utf8'));
    const scripts = (pkg as { scripts?: Record<string, string> }).scripts ?? {};
    expect(scripts['typecheck'], 'a project no script names is a project nothing checks').toContain(
      'tsconfig.e2e.json',
    );
  });

  it('keeps the DOM lib out of the node project, which is what makes the split bite', () => {
    expect(compilerOptions('tsconfig.node.json')['lib']).not.toContain('DOM');
  });

  it('keeps Node types out of the renderer project', () => {
    // The renderer is sandboxed and may never originate a filesystem path or an executable.
    // Withholding @types/node makes that a compile error rather than a review comment.
    const types = compilerOptions('tsconfig.web.json')['types'];
    expect(Array.isArray(types)).toBe(true);
    expect(types).not.toContain('node');
  });

  it('leaves DOM tests to the web project, which is the only one that carries the DOM lib', () => {
    // The mirror of the rule below. test/dom/** mounts into a document, and the node project
    // deliberately has no DOM lib — that absence is what stops a main- or preload-side test
    // passing against a global the sandboxed shell has never had.
    expect(readTsconfig('tsconfig.node.json')['exclude']).toContain('test/dom/**/*.ts');
    expect(readTsconfig('tsconfig.web.json')['include']).toContain('test/dom/**/*.ts');
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

describe('the testkit feature reaches the commands that matter', () => {
  // core::testing and core::corpus are gated on a default-off `testkit` feature, and the
  // integration tests that link them are gated to match. A bare `cargo test` therefore runs
  // 126 of 142 tests and still reports success. These assertions are what stops the flag from
  // being dropped from the two commands that are supposed to run everything.
  const repoRoot = fileURLToPath(new URL('..', import.meta.url));

  function repoFile(rel: string): string {
    return readFileSync(join(repoRoot, '..', rel), 'utf8');
  }

  it('keeps --features testkit on the root test script', () => {
    const pkg: unknown = JSON.parse(repoFile('package.json'));
    const scripts = (pkg as { scripts?: Record<string, string> }).scripts ?? {};
    expect(scripts['test']).toContain('cargo test');
    expect(scripts['test'], 'a bare cargo test skips every seam test').toContain(
      '--features testkit',
    );
  });

  it('keeps --all-features on the lint script, or the gated code is never linted', () => {
    const pkg: unknown = JSON.parse(repoFile('package.json'));
    const scripts = (pkg as { scripts?: Record<string, string> }).scripts ?? {};
    expect(scripts['lint']).toContain('--all-features');
  });

  it('keeps the flag on every cargo test CI runs', () => {
    const ci = repoFile('.github/workflows/ci.yml');
    // Per invocation, not as one fixed string. The whole-command form broke the moment `--locked`
    // was inserted between `cargo test` and `--features testkit`, reporting the loss of a flag
    // that had not moved — and it saw only the one job whose spelling it happened to carry.
    const invocations = ci
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => !line.startsWith('#') && line.includes('cargo test'));
    expect(invocations.length, 'CI runs no cargo test at all').toBeGreaterThan(0);
    for (const line of invocations) {
      expect(line, 'a bare cargo test skips every seam test').toContain('--features testkit');
      expect(line).toContain('--manifest-path core/Cargo.toml');
    }
    expect(ci).toContain('--all-targets --all-features -- -D warnings');
  });
});

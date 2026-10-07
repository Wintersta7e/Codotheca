import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';
import { afterEach, expect, it } from 'vitest';
import { bootstrap } from '../src/main/bootstrap';
import { bootFilePath, readBootFile, writeBootFile } from '../src/main/bootStore';
import { resolveDataDir } from '../src/main/paths';
import { registerShellServices } from '../src/main/shellServices';
import { IPC_CLEAR_PAINT_FAILURE } from '../src/shared/channels';

const made: string[] = [];

afterEach(() => {
  for (const dir of made.splice(0)) rmSync(dir, { recursive: true, force: true });
});

function temporary(prefix: string): string {
  const dir = mkdtempSync(join(tmpdir(), prefix));
  made.push(dir);
  return dir;
}

const file = fileURLToPath(new URL('../src/main/index.ts', import.meta.url));
const source = ts.createSourceFile(file, readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true);

function propertyValue(
  object: ts.ObjectLiteralExpression,
  name: string,
): ts.Expression | undefined {
  for (const p of object.properties) {
    if (p.name === undefined || !ts.isIdentifier(p.name) || p.name.text !== name) continue;
    if (ts.isPropertyAssignment(p)) return p.initializer;
    if (ts.isShorthandPropertyAssignment(p)) return p.name;
  }
  return undefined;
}

function declarations(name: string): ts.VariableDeclaration[] {
  const found: ts.VariableDeclaration[] = [];
  const visit = (node: ts.Node): void => {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === name) {
      found.push(node);
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return found;
}

/**
 * The directory an expression in `index.ts` names, for a launch with `userData` as Electron's
 * own directory and `env` as the process environment. Only the shapes the entry point uses are
 * understood; anything else throws rather than guessing.
 */
function evaluate(node: ts.Expression, userData: string, env: NodeJS.ProcessEnv): string {
  if (ts.isIdentifier(node)) {
    const found = declarations(node.text);
    const initializer = found.length === 1 ? found[0]?.initializer : undefined;
    if (initializer === undefined) {
      throw new Error(`index.ts declares ${node.text} ${String(found.length)} times`);
    }
    return evaluate(initializer, userData, env);
  }
  if (ts.isCallExpression(node) && ts.isIdentifier(node.expression)) {
    const [argument] = node.arguments;
    if (node.expression.text === 'resolveDataDir' && argument !== undefined) {
      if (!ts.isObjectLiteralExpression(argument))
        throw new Error('resolveDataDir takes a literal');
      const userDataPath = propertyValue(argument, 'userDataPath');
      const given = propertyValue(argument, 'env');
      if (userDataPath === undefined || given?.getText() !== 'process.env') {
        throw new Error(`resolveDataDir is given ${argument.getText()}`);
      }
      return resolveDataDir({ userDataPath: evaluate(userDataPath, userData, env), env });
    }
  }
  if (
    ts.isCallExpression(node) &&
    node.expression.getText() === 'app.getPath' &&
    node.arguments.length === 1 &&
    node.arguments[0] !== undefined &&
    ts.isStringLiteral(node.arguments[0]) &&
    node.arguments[0].text === 'userData'
  ) {
    return userData;
  }
  throw new Error(`index.ts names a boot.json directory this test cannot read: ${node.getText()}`);
}

/** Every dependency object in `index.ts` that reaches `boot.json`, and the directory it carries. */
function bootSites(userData: string, env: NodeJS.ProcessEnv): Map<string, string> {
  const sites = new Map<string, string>();
  const visit = (node: ts.Node): void => {
    if (ts.isObjectLiteralExpression(node) && propertyValue(node, 'readBoot') !== undefined) {
      const dir = propertyValue(node, 'dataDir') ?? propertyValue(node, 'userDataDir');
      if (dir === undefined)
        throw new Error(`a boot.json caller names no directory: ${node.getText()}`);
      const call = node.parent;
      const callee =
        ts.isCallExpression(call) && ts.isIdentifier(call.expression) ? call.expression.text : '?';
      const line = source.getLineAndCharacterOfPosition(node.getStart()).line + 1;
      sites.set(`${callee} (index.ts:${String(line)})`, evaluate(dir, userData, env));
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return sites;
}

it('AC-P4-48-24 with CODOTHECA_DATA_DIR set, the reset and the bootstrap use one boot.json', () => {
  const userData = temporary('codotheca-user-data-');
  const override = temporary('codotheca-data-dir-');
  const env: NodeJS.ProcessEnv = { CODOTHECA_DATA_DIR: override };

  const sites = bootSites(userData, env);
  // eslint-disable-next-line no-console -- the inventory is the evidence the check covered
  console.log(`boot.json callers in index.ts: ${String(sites.size)}`);
  const callees = [...sites.keys()].map((label) => label.split(' ')[0]);
  expect(callees).toEqual(expect.arrayContaining(['bootstrap', 'registerShellServices']));
  // One directory, and it is the one the override names: a diff here names both paths.
  expect(Object.fromEntries(sites)).toEqual(
    Object.fromEntries([...sites.keys()].map((label) => [label, override])),
  );

  const at = (callee: string): string => {
    const found = [...sites].find(([label]) => label.startsWith(`${callee} `));
    if (found === undefined) throw new Error(`index.ts never calls ${callee} with boot.json`);
    return found[1];
  };
  const launch = (): void => {
    bootstrap({
      argv: [],
      env,
      userDataDir: at('bootstrap'),
      registerSchemesAsPrivileged: () => undefined,
      disableHardwareAcceleration: () => undefined,
      readBoot: readBootFile,
      writeBoot: writeBootFile,
    });
  };

  // Two launches that never painted: the second reads the count the first wrote.
  launch();
  launch();
  expect(readBootFile(at('bootstrap')).paintFailCount).toBe(2);

  const handlers = new Map<string, (event: unknown, args: unknown) => unknown>();
  registerShellServices({
    handle: (channel, fn) => {
      handlers.set(channel, fn);
    },
    openExecutable: () => Promise.resolve(null),
    revealItem: () => undefined,
    dataDir: at('registerShellServices'),
    logPath: join(override, 'logs', 'codotheca.log'),
    statSync: () => ({ size: 0 }),
    request: () => Promise.resolve(null),
    readBoot: readBootFile,
    writeBoot: writeBootFile,
  });
  const reset = handlers.get(IPC_CLEAR_PAINT_FAILURE);
  expect(reset, 'the shell registers the paint-failure reset').toBeDefined();
  reset?.(null, undefined);

  expect(readBootFile(at('bootstrap')).paintFailCount).toBe(0);
  expect(existsSync(bootFilePath(userData))).toBe(false);
});

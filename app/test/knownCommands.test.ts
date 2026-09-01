import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from 'vitest';

const REPO = fileURLToPath(new URL('../..', import.meta.url));
const LOOP_ONLY: readonly string[] = ['app.hello_ack', 'app.shutdown'];

interface SchemaCommand {
  readonly name: string;
  readonly privileged?: boolean;
}

function isSchemaCommand(value: unknown): value is SchemaCommand {
  return (
    typeof value === 'object' &&
    value !== null &&
    'name' in value &&
    typeof value.name === 'string' &&
    (!('privileged' in value) || typeof value.privileged === 'boolean')
  );
}

function schemaCommands(): readonly SchemaCommand[] {
  const doc: unknown = JSON.parse(
    readFileSync(join(REPO, 'protocol/schema/protocol.json'), 'utf8'),
  );
  const commands =
    typeof doc === 'object' && doc !== null && 'commands' in doc ? doc.commands : null;
  if (!Array.isArray(commands) || commands.length === 0 || !commands.every(isSchemaCommand)) {
    throw new Error('protocol.json must declare a non-empty command array');
  }
  return commands;
}

function knownCommands(): string[] {
  const source = readFileSync(join(REPO, 'app/src/main/index.ts'), 'utf8');
  const block = /const\s+KNOWN_COMMANDS:\s*readonly\s+CommandName\[\]\s*=\s*\[([\s\S]*?)\];/u.exec(
    source,
  );
  const body = block?.[1];
  if (body === undefined) {
    throw new Error('KNOWN_COMMANDS is declared in app/src/main/index.ts');
  }

  const names: string[] = [];
  for (const match of body.matchAll(/'([^']+)'/gu)) {
    const name = match[1];
    if (name === undefined) throw new Error('KNOWN_COMMANDS contains an unreadable entry');
    names.push(name);
  }
  return names;
}

/**
 * Read the Rust literal's declared arity as well as its entries.
 *
 * An empty `[(&str, &str); 0] = []` must parse as zero. Treating a failed match as an empty
 * list would make every downstream comparison capable of passing without reading Rust at all.
 */
function unownedCommands(): string[] {
  const source = readFileSync(join(REPO, 'core/src/assembly/route.rs'), 'utf8');
  const block = /UNOWNED_COMMANDS:\s*\[\(&str,\s*&str\);\s*(\d+)\]\s*=\s*\[([\s\S]*?)\];/u.exec(
    source,
  );
  const arity = block?.[1];
  const body = block?.[2];
  if (arity === undefined || body === undefined) {
    throw new Error('UNOWNED_COMMANDS is declared in core/src/assembly/route.rs');
  }

  const names: string[] = [];
  for (const match of body.matchAll(/\("([^"]+)",\s*"[^"]+"\)/gu)) {
    const name = match[1];
    if (name === undefined) throw new Error('UNOWNED_COMMANDS contains an unreadable entry');
    names.push(name);
  }
  expect(names, "UNOWNED_COMMANDS's declared arity and entries disagree").toHaveLength(
    Number(arity),
  );
  return names;
}

test('every known command is a schema command with a core handler', () => {
  const schema = new Set(schemaCommands().map((command) => command.name));
  const unowned = new Set(unownedCommands());

  for (const name of knownCommands()) {
    expect(schema.has(name), `${name} is in KNOWN_COMMANDS but absent from the schema`).toBe(true);
    expect(unowned.has(name), `${name} is in KNOWN_COMMANDS with no core handler`).toBe(false);
    expect(
      LOOP_ONLY.includes(name),
      `${name} belongs to the shell's supervisor channel, not the renderer`,
    ).toBe(false);
  }
});

test('every answerable schema command is known', () => {
  const unowned = new Set(unownedCommands());
  const known = new Set(knownCommands());

  for (const { name } of schemaCommands()) {
    if (unowned.has(name) || LOOP_ONLY.includes(name)) continue;
    expect(known.has(name), `${name} has a handler and the bridge refuses it anyway`).toBe(true);
  }
});

test('the list is exactly the schema minus loop-only and unowned commands', () => {
  const known = knownCommands();
  const expected = schemaCommands().length - LOOP_ONLY.length - unownedCommands().length;

  expect(
    known,
    'KNOWN_COMMANDS must equal schema commands minus loop-only and unowned commands',
  ).toHaveLength(expected);
  expect(new Set(known).size, 'KNOWN_COMMANDS must not contain duplicates').toBe(known.length);
});

test('privileged commands are known even though the renderer door refuses them', () => {
  const privileged = schemaCommands()
    .filter((command) => command.privileged === true)
    .map((command) => command.name);
  const known = new Set(knownCommands());

  expect(privileged.length, 'the schema must identify privileged commands').toBeGreaterThan(0);
  for (const name of privileged) {
    expect(known.has(name), `${name} is privileged but still needs a core handler`).toBe(true);
  }
});

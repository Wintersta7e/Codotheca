import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from 'vitest';

/**
 * The shell subscribes to a hard-coded list of topics, and nothing asserted that the list covered
 * the schema.
 *
 * A topic the schema carries and the shell does not subscribe to delivers nothing: the surface
 * built on it renders empty for ever and no gate says so, because there is no failing call and no
 * error frame — it is discovered dark rather than red. That is the shape R88 was ruled on, and it
 * is the merge hazard `KNOWN_COMMANDS` next door is already covered for: two lanes each adding a
 * topic to the same line conflict, and a resolution taking either side silently drops the other.
 */
const REPO = fileURLToPath(new URL('../..', import.meta.url));

function schemaTopics(): string[] {
  const doc: unknown = JSON.parse(
    readFileSync(join(REPO, 'protocol/schema/protocol.json'), 'utf8'),
  );
  const topics = typeof doc === 'object' && doc !== null && 'topics' in doc ? doc.topics : null;
  if (typeof topics !== 'object' || topics === null || Array.isArray(topics)) {
    throw new Error('protocol.json must declare a topic map');
  }
  const names = Object.keys(topics);
  if (names.length === 0) throw new Error('protocol.json declares no topic');
  return names;
}

/**
 * Read the shell literal rather than importing it: the declaration is what a merge resolves, and
 * an import would compare the file against itself through a build step that can hide a drop.
 */
function shellTopics(): string[] {
  const source = readFileSync(join(REPO, 'app/src/main/index.ts'), 'utf8');
  const block = /const\s+TOPICS:\s*Topic\[\]\s*=\s*\[([\s\S]*?)\];/u.exec(source);
  const body = block?.[1];
  if (body === undefined) {
    throw new Error('TOPICS is declared in app/src/main/index.ts');
  }

  const names: string[] = [];
  for (const match of body.matchAll(/'([^']+)'/gu)) {
    const name = match[1];
    if (name === undefined) throw new Error('TOPICS contains an unreadable entry');
    names.push(name);
  }
  if (names.length === 0) throw new Error('TOPICS parsed as empty, which no build would be');
  return names;
}

test('the shell subscribes to every topic the schema declares', () => {
  const schema = schemaTopics();
  const shell = new Set(shellTopics());

  for (const name of schema) {
    expect(
      shell.has(name),
      `${name} is a schema topic the shell never subscribes to, so its surface renders dark`,
    ).toBe(true);
  }
});

test('the shell subscribes to no topic the schema does not carry', () => {
  const schema = new Set(schemaTopics());

  for (const name of shellTopics()) {
    expect(schema.has(name), `${name} is subscribed to and the schema does not declare it`).toBe(
      true,
    );
  }
});

test('the list is exactly the schema topics, with no duplicate', () => {
  const shell = shellTopics();

  expect(shell, 'TOPICS must equal the schema topic set').toHaveLength(schemaTopics().length);
  expect(new Set(shell).size, 'TOPICS must not contain duplicates').toBe(shell.length);
});

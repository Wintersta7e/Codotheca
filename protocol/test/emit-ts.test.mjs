import test from 'node:test';
import assert from 'node:assert/strict';
import { emitTypeScript, pascal } from '../lib/emit-ts.mjs';

const schema = {
  version: 2,
  errors: ['PROTOCOL', 'CORE_RESTARTED'],
  types: {
    ProjectId: { kind: 'id', repr: 'i64' },
    SceneHash: { kind: 'id', repr: 'String' },
    Presence: { kind: 'enum', variants: ['present', 'offline'] },
    Empty: { kind: 'struct', fields: {} },
    LocationRef: { kind: 'struct', fields: { id: 'ProjectId', pathDisplay: 'String' } },
    Row: {
      kind: 'struct',
      fields: {
        id: 'ProjectId',
        seenAt: 'Timestamp?',
        tags: '[String]',
        copies: '[LocationRef]?',
      },
    },
  },
  commands: [
    { name: 'app.shutdown', args: {}, returns: 'Empty' },
    { name: 'projects.get', args: { id: 'ProjectId' }, returns: 'Row' },
    { name: 'roots.add', args: { pathBytes: 'Bytes' }, returns: 'Empty', privileged: true },
    { name: 'projects.launch', args: { id: 'ProjectId' }, returns: 'Empty', idempotent: false },
  ],
  topics: { projects: { upserted: 'Row' } },
};

const out = emitTypeScript(schema);

test('pascal derives one identifier from a wire name', () => {
  assert.equal(pascal('app.hello_ack'), 'AppHelloAck');
  assert.equal(pascal('projects.setFlags'), 'ProjectsSetFlags');
});

test('ids are branded and carry their representation', () => {
  assert.match(out, /export type ProjectId = number & \{ readonly __brand: 'ProjectId' \};/);
  assert.match(out, /export type SceneHash = string & \{ readonly __brand: 'SceneHash' \};/);
});

test('enums are string-literal unions', () => {
  assert.match(out, /export type Presence = 'present' \| 'offline';/);
});

// A bare empty object type accepts 0 and "" and would let a malformed call through the contract.
test('a no-field shape is Record<string, never>, never an empty interface', () => {
  assert.match(out, /export type Empty = Record<string, never>;/);
  assert.match(out, /export type AppShutdownArgs = Record<string, never>;/);
  assert.doesNotMatch(out, /interface \w+ \{\s*\}/);
});

test('struct fields are readonly, nullable rather than optional, and arrays are readonly', () => {
  assert.match(out, /readonly id: ProjectId;/);
  assert.match(out, /readonly seenAt: number \| null;/);
  assert.match(out, /readonly tags: readonly string\[\];/);
  assert.match(out, /readonly copies: readonly LocationRef\[\] \| null;/);
  assert.doesNotMatch(out, /\?:/);
});

test('Bytes is the b64 tagging of §2.5', () => {
  assert.match(out, /export interface Bytes \{\n {2}readonly b64: string;\n\}/);
});

test('the command maps name every command', () => {
  assert.match(out, /'app\.shutdown': AppShutdownArgs;/);
  assert.match(out, /'projects\.get': Row;/);
});

test('event payloads are keyed topic/event', () => {
  assert.match(out, /'projects\/upserted': Row;/);
});

test('the privileged and non-idempotent sets are exported as const tuples', () => {
  assert.match(out, /export const PRIVILEGED_COMMANDS = \['roots\.add'\] as const;/);
  assert.match(out, /export const NON_IDEMPOTENT_COMMANDS = \['projects\.launch'\] as const;/);
});

test('emission is deterministic', () => {
  assert.equal(emitTypeScript(schema), out);
});

// The schema carries no topic until they are declared, so zero topics is a state the product
// reaches. Every other test here uses a fixture with one topic, which cannot see this: the
// degenerate forms are `export type Topic = ;`, which does not parse, and two empty interfaces.
test('a schema with no topics still emits parseable TypeScript', () => {
  const empty = emitTypeScript({ ...schema, topics: {} });
  assert.match(empty, /export type Topic = never;/);
  assert.match(empty, /export type TopicEvents = Record<string, never>;/);
  assert.match(empty, /export type EventPayloads = Record<string, never>;/);
  assert.doesNotMatch(empty, /interface \w+ \{\s*\}/);
  assert.doesNotMatch(empty, /=\s*;/);
});

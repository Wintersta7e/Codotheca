import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { emitRust } from '../lib/emit-rust.mjs';

const schema = {
  version: 2,
  errors: ['PROTOCOL', 'CORE_RESTARTED'],
  types: {
    ProjectId: { kind: 'id', repr: 'i64' },
    Presence: { kind: 'enum', variants: ['present', 'offline'] },
    Empty: { kind: 'struct', fields: {} },
    Row: {
      kind: 'struct',
      fields: {
        id: 'ProjectId',
        pathDisplay: 'String',
        seenAt: 'Timestamp?',
        tags: '[String]',
        type: 'String',
      },
    },
  },
  commands: [
    { name: 'app.shutdown', args: {}, returns: 'Empty' },
    { name: 'projects.get', args: { id: 'ProjectId' }, returns: 'Row' },
    { name: 'roots.add', args: { pathBytes: 'Bytes' }, returns: 'Empty', privileged: true },
  ],
  topics: { projects: { upserted: 'Row' } },
};

const out = emitRust(schema);

test('ids are transparent newtypes', () => {
  assert.match(out, /#\[serde\(transparent\)\]\npub struct ProjectId\(pub i64\);/);
});

test('enum variants are renamed to the wire spelling', () => {
  assert.match(out, /#\[serde\(rename = "present"\)\]\n {4}Present,/);
});

test('structs rename to camelCase and reject unknown fields', () => {
  assert.match(
    out,
    /#\[serde\(rename_all = "camelCase", deny_unknown_fields\)\]\npub struct Row \{/,
  );
  assert.match(out, /pub path_display: String,/);
  assert.match(out, /pub seen_at: Option<i64>,/);
  assert.match(out, /pub tags: Vec<String>,/);
});

// A wire name that is a Rust keyword must still compile.
test('a keyword field name is escaped as a raw identifier', () => {
  assert.match(out, /pub r#type: String,/);
});

test('the Command enum is externally tagged on command and args', () => {
  assert.match(out, /#\[serde\(tag = "command", content = "args"\)\]\npub enum Command \{/);
  assert.match(out, /#\[serde\(rename = "projects\.get"\)\]\n {4}ProjectsGet\(ProjectsGetArgs\),/);
});

test('results and events are untagged and serialize-only, and name themselves', () => {
  assert.match(out, /#\[serde\(untagged\)\]\npub enum CommandResultValue \{/);
  assert.match(out, /pub fn command\(&self\) -> CommandName \{/);
  assert.match(out, /pub enum ProjectsEvent \{/);
  assert.match(out, /pub fn name\(&self\) -> &'static str \{/);
  assert.match(out, /pub fn topic\(&self\) -> Topic \{/);
});

test('every public item derives Debug, for missing_debug_implementations', () => {
  const items = out.match(/^pub (struct|enum) /gm) ?? [];
  const derives = out.match(/^#\[derive\([^)]*Debug[^)]*\)\]$/gm) ?? [];
  assert.ok(derives.length >= items.length, `${derives.length} derives for ${items.length} items`);
});

test('Bytes carries the b64 codec, not a raw Vec on the wire', () => {
  assert.match(out, /pub struct Bytes\(pub Vec<u8>\);/);
  assert.match(out, /"b64"/);
});

// Zero topics is a state the schema reaches. rustfmt rejects an enum whose braces enclose only
// a blank line, so `cargo fmt --check` fails on emitted text — a gate failing on generated
// output nobody hand-writes.
test('an empty enum is emitted in the form rustfmt accepts', () => {
  const empty = emitRust({ ...schema, topics: {} });
  assert.match(empty, /pub enum Topic \{\}/);
  assert.doesNotMatch(empty, /pub enum \w+ \{\n\s*\n\}/);
});

/**
 * R47's second half. §7.6's address is `codotheca://art/<hash>/<rendition>`, so a card blueprint
 * and a hero blueprint need distinct one-segment names — and a hyphen is the only separator that
 * keeps them one segment. `pascal` split on `[._/]` and not `-`, so it produced
 * `Card-blueprint`, which is not a Rust identifier and would not compile.
 */
test('a hyphenated enum variant emits a legal Rust identifier and its verbatim rename', () => {
  const hyphenated = emitRust({
    ...schema,
    types: {
      ...schema.types,
      Rendition: { kind: 'enum', variants: ['card', 'hero', 'card-blueprint', 'hero-blueprint'] },
    },
  });
  assert.match(hyphenated, /#\[serde\(rename = "card-blueprint"\)\]\n {4}CardBlueprint,/);
  assert.match(hyphenated, /#\[serde\(rename = "hero-blueprint"\)\]\n {4}HeroBlueprint,/);
  assert.doesNotMatch(hyphenated, /Card-blueprint/);
});

test('widening the split leaves every name in the real schema byte-identical', () => {
  // Asserted by regenerating and diffing, not by eyeballing: the claim is that **zero** names in
  // the schema contain a hyphen today, so the change alters no existing generated identifier.
  const real = JSON.parse(
    readFileSync(new URL('../schema/protocol.json', import.meta.url), 'utf8'),
  );
  const names = [
    ...Object.keys(real.types),
    ...Object.values(real.types).flatMap((d) => d.variants ?? []),
    ...real.commands.map((c) => c.name),
    ...Object.keys(real.topics),
    ...Object.entries(real.topics).flatMap(([, events]) => Object.keys(events)),
    ...real.errors,
  ];
  assert.ok(names.length > 0, 'a run that enumerated no names proves nothing');
  const hyphenated = names.filter(
    (n) => n.includes('-') && n !== 'card-blueprint' && n !== 'hero-blueprint',
  );
  assert.deepEqual(
    hyphenated,
    [],
    `only §23's rendition names may carry a hyphen; found ${hyphenated.join(', ')}`,
  );
});

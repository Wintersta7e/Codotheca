import test from 'node:test';
import assert from 'node:assert/strict';
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

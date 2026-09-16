import test from 'node:test';
import assert from 'node:assert/strict';
import { parseTypeExpr, validateSchema, SCALARS } from '../lib/schema.mjs';

const base = () => ({
  version: 2,
  errors: ['PROTOCOL'],
  types: {
    ProjectId: { kind: 'id', repr: 'i64' },
    Empty: { kind: 'struct', fields: {} },
    Presence: { kind: 'enum', variants: ['present', 'offline'] },
    Row: { kind: 'struct', fields: { id: 'ProjectId', tags: '[String]', seenAt: 'Timestamp?' } },
  },
  commands: [{ name: 'projects.get', args: { id: 'ProjectId' }, returns: 'Row' }],
  topics: { projects: { upserted: 'Row' } },
});

test('type expressions parse in all four forms', () => {
  assert.deepEqual(parseTypeExpr('ProjectId'), {
    base: 'ProjectId',
    array: false,
    nullable: false,
  });
  assert.deepEqual(parseTypeExpr('ProjectId?'), {
    base: 'ProjectId',
    array: false,
    nullable: true,
  });
  assert.deepEqual(parseTypeExpr('[ProjectId]'), {
    base: 'ProjectId',
    array: true,
    nullable: false,
  });
  assert.deepEqual(parseTypeExpr('[ProjectId]?'), {
    base: 'ProjectId',
    array: true,
    nullable: true,
  });
});

test('an unbalanced or nested array is not a type expression', () => {
  assert.throws(() => parseTypeExpr('[ProjectId'), /not a type expression/);
  assert.throws(() => parseTypeExpr('[[ProjectId]]'), /not a type expression/);
});

test('Timestamp and Bytes are scalars', () => {
  assert.ok(SCALARS['Timestamp']);
  assert.ok(SCALARS['Bytes']);
});

test('a valid schema passes', () => {
  validateSchema(base());
});

test('an undeclared type reference is rejected by name', () => {
  const s = base();
  s.commands[0].returns = 'Ghost';
  assert.throws(() => validateSchema(s), /projects\.get.*Ghost/s);
});

test('a duplicate command name is rejected', () => {
  const s = base();
  s.commands.push({ name: 'projects.get', args: {}, returns: 'Empty' });
  assert.throws(() => validateSchema(s), /duplicate command/);
});

test('a topic event with no payload type is rejected', () => {
  const s = base();
  s.topics.projects.merged = 'Ghost';
  assert.throws(() => validateSchema(s), /projects\/merged.*Ghost/s);
});

// §2.4: the renderer may never originate a filesystem path or an executable. Bytes travelling
// into the core is exactly that, so it is admissible only behind the privileged flag.
test('an unprivileged command taking Bytes is rejected', () => {
  const s = base();
  s.commands.push({ name: 'roots.add', args: { pathBytes: 'Bytes' }, returns: 'Empty' });
  assert.throws(() => validateSchema(s), /roots\.add.*privileged/s);
});

test('an unprivileged command mutating the filesystem is rejected', () => {
  const s = base();
  s.commands.push({
    name: 'locations.uninstall',
    args: { id: 'ProjectId' },
    returns: 'Empty',
    mutatesFilesystem: true,
  });
  assert.throws(() => validateSchema(s), /locations\.uninstall.*privileged/s);
});

test('a privileged command needs Bytes or filesystem mutation', () => {
  const s = base();
  const command = {
    name: 'roots.add',
    args: { id: 'ProjectId' },
    returns: 'Empty',
    privileged: true,
  };
  s.commands.push(command);
  assert.throws(() => validateSchema(s), /roots\.add.*Bytes/s);

  command.mutatesFilesystem = true;
  assert.doesNotThrow(() => validateSchema(s));
});

test('ErrorCode may not be hand-declared; it is synthesised from errors', () => {
  const s = base();
  s.types.ErrorCode = { kind: 'enum', variants: ['NOPE'] };
  assert.throws(() => validateSchema(s), /ErrorCode/);
});

// The two rules about ErrorCode pull in opposite directions and both are needed: it cannot be
// declared, and ErrorFrame.code must still be able to name it. Rejecting the reference would
// make the error frame — the one shape every failure crosses as — undeclarable.
test('ErrorCode is referenceable even though it is undeclarable', () => {
  const s = base();
  s.types.ErrorFrame = { kind: 'struct', fields: { code: 'ErrorCode', message: 'String' } };
  validateSchema(s);
});

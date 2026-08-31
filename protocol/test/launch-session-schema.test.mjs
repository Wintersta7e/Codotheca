import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSchema, parseTypeExpr } from '../lib/schema.mjs';

const schema = loadSchema(new URL('../schema/protocol.json', import.meta.url).pathname);
const command = (name) => schema.commands.find((c) => c.name === name);

test('session.focus exists, is unprivileged and returns Empty', () => {
  const c = command('session.focus');
  assert.ok(c, 'session.focus is absent from the command list');
  assert.equal(c.privileged ?? false, false);
  assert.equal(c.returns, 'Empty');
  assert.equal(c.args.projectId, 'ProjectId?');
});

test('targets.setDefault carries the whole scope triple and the target', () => {
  const c = command('targets.setDefault');
  assert.deepEqual(c.args, {
    targetId: 'TargetId',
    projectId: 'ProjectId?',
    locationId: 'LocationId?',
    language: 'String?',
  });
});

test('targets.upsert takes the executable as tagged bytes, never a string', () => {
  const c = command('targets.upsert');
  assert.equal(c.privileged, true);
  assert.equal(c.args.execBytes, 'Bytes');
  assert.equal(c.args.argv, '[String]');
  assert.ok(!('cwdMode' in c.args), 'a hand-added target gets CwdMode::Location, not a control');
});

test('projects.launch names the copy as well as the project and the target', () => {
  assert.deepEqual(command('projects.launch').args, {
    projectId: 'ProjectId',
    locationId: 'LocationId',
    targetId: 'TargetId',
  });
});

test('the launch value types exist and are shaped as the shell reads them', () => {
  assert.deepEqual(schema.types.TargetKind.variants, [
    'editor', 'terminal', 'file_manager', 'git_client',
  ]);
  assert.deepEqual(schema.types.VerifyState.variants, [
    'unverified', 'ok', 'missing', 'not_executable',
  ]);
  assert.deepEqual(schema.types.CwdMode.variants, ['location', 'none']);
  assert.deepEqual(schema.types.TargetTier.variants, [
    'project', 'location', 'language', 'global',
  ]);
  assert.equal(schema.types.TargetList.fields.rows, '[TargetRow]');
  assert.equal(schema.types.TargetList.fields.resolved, 'ResolvedTarget?');
  assert.equal(schema.types.TargetVerification.fields.verifyState, 'VerifyState');
});

test('no launch type leaks an executable or an argv to the renderer', () => {
  for (const name of ['TargetRow', 'TargetList', 'TargetVerification', 'ResolvedTarget']) {
    for (const field of Object.keys(schema.types[name].fields)) {
      assert.ok(!/^(exec|execBytes|exec_bytes|argv|args|env)$/.test(field),
        `${name}.${field} would put an executable or argv on the wire`);
    }
  }
});

test('the three session events are shaped, and closure reasons are the enum §1.6 states', () => {
  assert.deepEqual(schema.types.CloseReason.variants, [
    'stop', 'idle', 'process_exit', 'app_exit', 'crash', 'orphaned',
  ]);
  assert.deepEqual(schema.types.ClosedBy.variants, [
    'idle', 'session_end', 'app_exit', 'crash',
  ]);
  assert.equal(schema.topics.session.started, 'SessionStarted');
  assert.equal(schema.types.SessionStarted.fields.session, 'SessionRef');
  assert.equal(schema.types.SessionEnded.fields.session, 'SessionRef');
  const seg = schema.types.SegmentClosed.fields;
  assert.equal(seg.sessionId, 'SessionId');
  assert.equal(seg.creditedSeconds, 'i64');
  assert.equal(seg.sessionCreditedSeconds, 'i64');
  assert.equal(seg.closedBy, 'ClosedBy');
});

test('every type expression this task added parses', () => {
  for (const name of ['TargetList', 'TargetVerification', 'SegmentClosed']) {
    for (const expr of Object.values(schema.types[name].fields)) {
      assert.doesNotThrow(() => parseTypeExpr(expr), `${name}: ${expr}`);
    }
  }
});

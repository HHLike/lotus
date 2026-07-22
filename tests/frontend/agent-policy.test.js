'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const AgentPolicy = require('../../frontend/agent-policy.js');

test('agent state ends with the command and does not leak to ordinary commands', () => {
  let runtime = AgentPolicy.createRuntime();
  runtime = AgentPolicy.startCommand(runtime, 'pi', true);
  assert.deepEqual(
    { busy: runtime.busy, cmd: runtime.cmd, isAgent: runtime.isAgent },
    { busy: true, cmd: 'pi', isAgent: true }
  );

  const key = AgentPolicy.completionKey('pi', 0, 12000);
  const finished = AgentPolicy.finishCommand(runtime, key);
  assert.equal(finished.duplicate, false);
  assert.equal(finished.wasAgent, true);
  assert.deepEqual(finished.runtime, {
    busy: false,
    cmd: '',
    isAgent: false,
    lastCompletionKey: key,
  });

  runtime = AgentPolicy.startCommand(finished.runtime, 'echo ok', false);
  assert.equal(runtime.isAgent, false);
  assert.equal(runtime.lastCompletionKey, '');
});

test('a duplicate completion key is suppressed', () => {
  const key = AgentPolicy.completionKey('pi', 0, 12000);
  const first = AgentPolicy.finishCommand(
    AgentPolicy.startCommand(AgentPolicy.createRuntime(), 'pi', true),
    key
  );
  const duplicate = AgentPolicy.finishCommand(first.runtime, key);

  assert.equal(first.duplicate, false);
  assert.equal(duplicate.duplicate, true);
});

test('foreground completion has no retained badge or notification', () => {
  assert.deepEqual(
    AgentPolicy.completionDisposition({ active: true, durationMs: 30000, ok: true }),
    { badge: null, notify: false }
  );
});

test('short background completion keeps a badge without notifying', () => {
  assert.deepEqual(
    AgentPolicy.completionDisposition({ active: false, durationMs: 9999, ok: true }),
    { badge: 'done', notify: false }
  );
});

test('ten-second background completion keeps result badge and notifies', () => {
  assert.deepEqual(
    AgentPolicy.completionDisposition({ active: false, durationMs: 10000, ok: false }),
    { badge: 'error', notify: true }
  );
});

test('missing or invalid duration never notifies', () => {
  for (const durationMs of [undefined, null, NaN, Infinity, -1]) {
    assert.equal(
      AgentPolicy.completionDisposition({ active: false, durationMs, ok: true }).notify,
      false
    );
  }
});

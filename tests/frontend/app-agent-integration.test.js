'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const root = path.resolve(__dirname, '../..');
const html = fs.readFileSync(path.join(root, 'frontend/index.html'), 'utf8');
const app = fs.readFileSync(path.join(root, 'frontend/app.js'), 'utf8');

test('agent policy loads before the application script', () => {
  const policyIndex = html.indexOf('<script src="agent-policy.js"></script>');
  const appIndex = html.indexOf('<script src="app.js"></script>');
  assert.ok(policyIndex >= 0);
  assert.ok(policyIndex < appIndex);
});

test('app delegates lifecycle and completion decisions to AgentPolicy', () => {
  assert.match(app, /AgentPolicy\.createRuntime\(\)/);
  assert.match(app, /AgentPolicy\.startCommand\(/);
  assert.match(app, /AgentPolicy\.finishCommand\(/);
  assert.match(app, /AgentPolicy\.completionDisposition\(/);
});

test('tab titles do not create sticky agent state', () => {
  assert.doesNotMatch(app, /isAgent:\s*isAgentCommand\(title\)/);
  assert.doesNotMatch(app, /isAgentCommand\(msg\.title\)/);
});

test('legacy unconditional agent notification condition is removed', () => {
  assert.doesNotMatch(app, /isAgent\s*\|\|\s*\(notFocused/);
});

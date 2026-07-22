const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

let NotificationSettings;
try {
  NotificationSettings = require('./notification-settings.js');
} catch (_) {
  // The first RED run intentionally happens before the module exists.
}

test('agent completion notifications default to enabled and can be disabled', () => {
  assert.equal(typeof NotificationSettings?.shouldSend, 'function');
  assert.equal(
    NotificationSettings.shouldSend({}, 'agent', true),
    true
  );
  assert.equal(
    NotificationSettings.shouldSend({ agent_notifications_enabled: false }, 'agent', true),
    false
  );
});

test('ordinary command completion notifications default to disabled and can be enabled', () => {
  assert.equal(typeof NotificationSettings?.shouldSend, 'function');
  assert.equal(NotificationSettings.shouldSend({}, 'command', true), false);
  assert.equal(
    NotificationSettings.shouldSend({ command_notifications_enabled: true }, 'command', true),
    true
  );
  assert.equal(
    NotificationSettings.shouldSend({ command_notifications_enabled: false }, 'command', true),
    false
  );
});

test('agent eligibility is respected while enabled command notifications are explicit opt-in', () => {
  assert.equal(
    NotificationSettings.shouldSend({ agent_notifications_enabled: true }, 'agent', false),
    false
  );
  assert.equal(
    NotificationSettings.shouldSend({ command_notifications_enabled: true }, 'command', false),
    true
  );
  assert.equal(NotificationSettings.shouldSend({}, 'unknown', true), false);
});

test('settings UI and dispatch use two independent notification preferences', () => {
  const html = fs.readFileSync(path.join(__dirname, 'index.html'), 'utf8');
  const app = fs.readFileSync(path.join(__dirname, 'app.js'), 'utf8');

  assert.match(html, /id="setting-agent-notifications-enabled"/);
  assert.match(html, /id="setting-command-notifications-enabled"/);
  assert.doesNotMatch(html, /id="setting-notifications-enabled"/);
  assert.match(app, /agent_notifications_enabled\s*:/);
  assert.match(app, /command_notifications_enabled\s*:/);
  assert.match(app, /kind:\s*notificationKind/);
  assert.match(app, /type:\s*'desktop_notify'[\s\S]{0,180}tab_id:\s*tabId/);
  assert.doesNotMatch(app, /\bnotifications_enabled\s*:/);
});

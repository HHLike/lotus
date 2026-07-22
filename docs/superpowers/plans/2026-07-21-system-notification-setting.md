# System Notification Setting Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a persistent Settings switch that lets users disable all Lotus desktop completion notifications.

**Architecture:** Store `notifications_enabled` in the existing TOML configuration with a backward-compatible default of `true`, expose it through `ConfigPayload`, push the saved value to the frontend during startup, and render it as a checkbox in Settings. Gate notifications in both the frontend policy and Rust IPC handler so disabling the switch takes effect immediately and remains enforced even if a stale frontend requests a notification.

**Tech Stack:** Rust 2024, Serde/TOML, browser JavaScript, Node.js built-in test runner, HTML/CSS.

## Global Constraints

- Existing config files without `notifications_enabled` must load with notifications enabled.
- Disabling the switch must suppress Agent and ordinary-command desktop notifications.
- Tab busy/done/error badges must continue to work when notifications are disabled.
- Resetting settings restores notifications to enabled; saving persists the selected value.
- No new runtime dependencies.

---

### Task 1: Persist and enforce the notification preference

**Files:**
- Modify: `src/config.rs`
- Modify: `src/ipc.rs`
- Modify: `src/main.rs`
- Test: `src/config.rs`

**Interfaces:**
- Consumes: optional TOML field `notifications_enabled` and `ConfigPayload.notifications_enabled`.
- Produces: `Config.notifications_enabled: bool`, defaulting to `true`, plus a backend guard around `desktop_notify`.

- [x] **Step 1: Write failing backward-compatibility tests**

```rust
#[test]
fn old_config_defaults_notifications_to_enabled() {
    let cfg: Config = toml::from_str("theme = \"lotus\"").unwrap();
    assert!(cfg.notifications_enabled);
}

#[test]
fn notifications_can_be_disabled_and_round_trip() {
    let mut cfg = Config::default();
    cfg.notifications_enabled = false;
    let encoded = toml::to_string(&cfg).unwrap();
    let decoded: Config = toml::from_str(&encoded).unwrap();
    assert!(!decoded.notifications_enabled);
}
```

- [x] **Step 2: Run the tests to verify RED**

Run: `cargo test config::tests -- --nocapture`
Expected: compilation fails because `Config::notifications_enabled` does not exist.

- [x] **Step 3: Add the config field and IPC mapping**

Add `#[serde(default = "default_notifications_enabled")] pub notifications_enabled: bool` to `Config`, return `true` from the default function, and include it in `Default`, `ConfigPayload`, `config_to_payload`, and the `SaveConfig` assignment.

- [x] **Step 4: Guard backend desktop notification requests**

```rust
ClientMessage::DesktopNotify { title, body } => {
    if s.config.notifications_enabled {
        desktop_notify(&title, &body);
    }
}
```

- [x] **Step 5: Run targeted Rust tests to verify GREEN**

Run: `cargo test config::tests -- --nocapture`
Expected: both notification config tests pass.

### Task 2: Add and wire the Settings switch

**Files:**
- Create: `frontend/notification-settings.js`
- Create: `frontend/notification-settings.test.js`
- Modify: `frontend/index.html`
- Modify: `frontend/styles.css`
- Modify: `frontend/app.js`

**Interfaces:**
- Consumes: `_currentConfig.notifications_enabled`, checkbox state, and the existing notification eligibility decision.
- Produces: global/CommonJS `NotificationSettings.shouldSend(config, eligible)` and a saved boolean form value.

- [x] **Step 1: Write the failing frontend policy tests**

```javascript
test('disabled notifications suppress otherwise eligible completion alerts', () => {
  assert.equal(NotificationSettings.shouldSend({ notifications_enabled: false }, true), false);
});

test('old or enabled config allows eligible completion alerts', () => {
  assert.equal(NotificationSettings.shouldSend({}, true), true);
  assert.equal(NotificationSettings.shouldSend({ notifications_enabled: true }, true), true);
  assert.equal(NotificationSettings.shouldSend({ notifications_enabled: true }, false), false);
});
```

- [x] **Step 2: Run the frontend tests to verify RED**

Run: `node --test frontend/notification-settings.test.js`
Expected: FAIL because the policy module does not exist.

- [x] **Step 3: Implement the pure notification gate**

```javascript
function shouldSend(config, eligible) {
  return config?.notifications_enabled !== false && Boolean(eligible);
}
```

- [x] **Step 4: Add the Settings switch and form wiring**

Add a checked checkbox with id `setting-notifications-enabled`, populate it with `cfg.notifications_enabled !== false`, update `_currentConfig` on change, include the boolean in `collectFormConfig`, and set it to `true` in Reset defaults. Load `notification-settings.js` before `app.js`.

- [x] **Step 5: Gate frontend completion notifications without changing badges**

Wrap only the `sendToBackend({ type: 'desktop_notify', ... })` branch with `NotificationSettings.shouldSend(_currentConfig, eligible)`; keep the existing badge calls outside the gate.

- [x] **Step 6: Run frontend tests to verify GREEN**

Run: `node --test frontend/notification-settings.test.js`
Expected: all notification-setting tests pass.

### Task 3: Verify the integrated change

**Files:**
- Verify all files above plus the existing terminal-output changes in the working tree.

**Interfaces:**
- Consumes: saved Settings form data and command-completion notification requests.
- Produces: no desktop notification when disabled, with unchanged terminal badges and output flow.

- [x] **Step 1: Run syntax and unit tests**

Run: `node --check frontend/app.js frontend/notification-settings.js && node --test frontend/*.test.js`
Expected: all frontend tests pass.

Run: `cargo test -- --nocapture`
Expected: all Rust tests pass.

- [x] **Step 2: Build and review scope**

Run: `cargo build && git diff --check && git status --short`
Expected: build succeeds, no whitespace errors, and only the terminal-output work plus this notification setting are modified.

# Split Completion Notifications Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users enable Agent CLI completion notifications without also enabling ordinary-command completion notifications.

**Architecture:** Replace the single notification preference with two persisted booleans, defaulting Agent notifications on and ordinary-command notifications off. Classify each completion in the frontend, include the category in desktop-notification IPC, and enforce the matching preference in both the frontend policy and Rust backend.

**Tech Stack:** Rust 2024, serde/TOML/JSON, plain browser JavaScript, Node.js test runner, GTK/WebKitGTK.

## Global Constraints

- Preserve unrelated terminal-output/backpressure changes already present in the worktree.
- Existing config files without either new field must load with Agent notifications enabled and ordinary-command notifications disabled.
- Agent and ordinary-command completion settings must be independently persisted and enforced.
- Tab busy/done/error badges must not depend on either notification preference.
- Unknown notification categories must be suppressed.

---

### Task 1: Define the two-category frontend notification policy

**Files:**
- Modify: `frontend/notification-settings.test.js`
- Modify: `frontend/notification-settings.js`

**Interfaces:**
- Consumes: config fields `agent_notifications_enabled`, `command_notifications_enabled`; notification kind `agent | command`; existing eligibility boolean.
- Produces: `NotificationSettings.shouldSend(config, kind, eligible): boolean`.

- [ ] **Step 1: Write failing policy tests**

Add tests proving that Agent notifications default to enabled, ordinary-command notifications default to disabled, both can be independently overridden, ineligible completions are suppressed, and unknown categories are suppressed.

- [ ] **Step 2: Run the focused test and verify RED**

Run: `node --test frontend/notification-settings.test.js`

Expected: FAIL because the existing two-argument global gate does not distinguish notification kinds.

- [ ] **Step 3: Implement the minimal category-aware gate**

Implement `shouldSend(config, kind, eligible)` with an early eligibility check, Agent default-on behavior, ordinary-command opt-in behavior, and a false fallback for unknown kinds.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run: `node --test frontend/notification-settings.test.js`

Expected: all notification policy tests pass.

### Task 2: Persist and enforce both preferences in Rust

**Files:**
- Modify: `src/config.rs`
- Modify: `src/ipc.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: TOML fields `agent_notifications_enabled`, `command_notifications_enabled`; JSON notification kind `agent | command`.
- Produces: matching fields in `Config` and `ConfigPayload`; `NotificationKind`; backend `notification_enabled(&Config, NotificationKind) -> bool` enforcement.

- [ ] **Step 1: Write failing config, IPC, and backend policy tests**

Cover defaults, independent round trips, JSON category parsing/defaulting, startup payload propagation, and independent backend category decisions.

- [ ] **Step 2: Run Rust tests and verify RED**

Run: `cargo test`

Expected: compilation fails because the two settings and `NotificationKind` do not exist.

- [ ] **Step 3: Implement the two config fields and typed notification category**

Replace `notifications_enabled` with `agent_notifications_enabled` (default `true`) and `command_notifications_enabled` (default `false`) across `Config`, `ConfigPayload`, save handling, and startup payload creation. Add an IPC `NotificationKind` enum whose missing value defaults to `command` for compatibility.

- [ ] **Step 4: Enforce the category-specific preference**

Include `kind` in `ClientMessage::DesktopNotify` and route it through `notification_enabled` before calling `desktop_notify`.

- [ ] **Step 5: Run Rust tests and verify GREEN**

Run: `cargo test`

Expected: all Rust tests pass; only the repository's pre-existing dead-code warnings remain.

### Task 3: Wire the two Settings switches and completion categories

**Files:**
- Modify: `frontend/index.html`
- Modify: `frontend/app.js`
- Modify: `README.md`

**Interfaces:**
- Consumes: the two config fields and `NotificationSettings.shouldSend(config, kind, eligible)`.
- Produces: two independent checkboxes; categorized `desktop_notify` messages; documented config fields.

- [ ] **Step 1: Add static integration assertions before wiring**

Extend the frontend test file to inspect `index.html` and `app.js`, asserting both checkbox IDs, both collected config keys, and the notification `kind` payload exist while the old single checkbox/config key is absent.

- [ ] **Step 2: Run the focused test and verify RED**

Run: `node --test frontend/notification-settings.test.js`

Expected: FAIL because the UI and app still use `notifications_enabled`.

- [ ] **Step 3: Wire the Settings form and notification dispatch**

Render `setting-agent-notifications-enabled` checked and `setting-command-notifications-enabled` unchecked. Normalize incoming config, update the cache on change, restore both defaults on Reset, collect both values on Save, classify completions as `agent` or `command`, pass the category to the policy, and include it in `desktop_notify` IPC.

- [ ] **Step 4: Update user-facing documentation**

Describe the two independent notification switches and add both TOML keys to the README configuration example.

- [ ] **Step 5: Run frontend tests and syntax checks**

Run: `node --check frontend/app.js frontend/notification-settings.js`

Run: `node --test frontend/*.test.js`

Expected: syntax checks and all frontend tests pass.

### Task 4: Full verification

**Files:**
- Verify only.

**Interfaces:**
- Consumes: all implementation changes.
- Produces: fresh evidence that the combined worktree still builds and tests.

- [ ] **Step 1: Format and inspect the diff**

Run: `cargo fmt --check`

Run: `git diff --check`

Expected: both commands exit successfully.

- [ ] **Step 2: Run the complete test suites**

Run: `node --test frontend/*.test.js`

Run: `cargo test`

Expected: all tests pass.

- [ ] **Step 3: Compile the application**

Run: `cargo check`

Expected: compilation succeeds with no new warnings.

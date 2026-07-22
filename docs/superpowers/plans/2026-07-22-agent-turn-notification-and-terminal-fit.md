# Agent Turn Notification and Terminal Fit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Notify when a persistent Agent CLI finishes a work turn, keep ordinary command notifications independently useful, and render the terminal's final row completely.

**Architecture:** Parse the standard OSC 9;4 terminal progress protocol in the PTY bridge and represent start/clear transitions as Agent lifecycle events. Instrument Pi sessions launched from Lotus with a generated extension that emits OSC 9;4 on `agent_start` and `agent_settled`; the backend de-duplicates transitions and sends the Agent notification without waiting for the Pi process to exit. Keep terminal gutter on `.xterm`, where FitAddon accounts for it.

**Tech Stack:** Rust 2024, serde, portable-pty, Bash/JavaScript-generated integrations, plain frontend JavaScript/CSS, Node.js tests.

## Global Constraints

- Preserve unrelated dirty-worktree changes.
- Agent notifications must fire on a completed Agent turn while the CLI process remains alive.
- Ordinary command notifications must remain controlled only by their own switch.
- Repeated OSC 9;4 keepalive events must not create duplicate notifications.
- The bottom terminal row must be fully visible at normal and maximized sizes.

---

### Task 1: Correct FitAddon gutter accounting

**Files:**
- Modify: `frontend/terminal-output.test.js`
- Modify: `frontend/styles.css`

**Interfaces:**
- Consumes: FitAddon behavior that subtracts `.xterm` padding.
- Produces: a 4px terminal gutter without overestimating visible rows.

- [x] **Step 1: Add a failing CSS ownership test**
- [x] **Step 2: Verify the test fails with padding on `.terminal-pane`**
- [x] **Step 3: Move the gutter to `.xterm`**
- [x] **Step 4: Verify the focused test passes**

### Task 2: Parse and de-duplicate Agent progress events

**Files:**
- Modify: `src/shell_integration.rs`
- Modify: `src/term/manager.rs`
- Modify: `src/ipc.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: OSC 9 payloads `4;3` (active) and `4;0`/`4;0;` (clear).
- Produces: `CommandEvent::AgentStart`, `CommandEvent::AgentEnd`, `TermEvent::AgentProgress`, and `ServerMessage::AgentProgress`.

- [ ] **Step 1: Add failing parser and lifecycle tests**
- [ ] **Step 2: Verify Rust tests fail because progress events are unsupported**
- [ ] **Step 3: Parse OSC 9;4 and forward lifecycle transitions**
- [ ] **Step 4: Track active Agent turns per tab and notify only on active-to-clear**
- [ ] **Step 5: Verify focused and full Rust tests pass**

### Task 3: Emit progress events from Pi launched by Lotus

**Files:**
- Modify: `src/shell_integration.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: Pi extension events `agent_start`, `agent_settled`, and `session_shutdown`.
- Produces: `~/.local/share/lotus/pi-notification-extension.js` and an instrumented `pi --extension <path>` launch command.

- [ ] **Step 1: Add failing generated-extension and command-instrumentation tests**
- [ ] **Step 2: Verify tests fail before the extension/helper exists**
- [ ] **Step 3: Generate the Pi extension and inject it into interactive Pi launches**
- [ ] **Step 4: Verify tests pass and non-Pi commands remain unchanged**

### Task 4: Make enabled ordinary-command notifications observable

**Files:**
- Modify: `frontend/notification-settings.test.js`
- Modify: `frontend/app.js`
- Modify: `frontend/index.html`

**Interfaces:**
- Consumes: `command_notifications_enabled` and ordinary `command_finished` events.
- Produces: a notification for each completed ordinary command when the user explicitly enables the switch.

- [ ] **Step 1: Add a failing notification eligibility test**
- [ ] **Step 2: Remove the hidden foreground/5-second restriction from the opt-in command category**
- [ ] **Step 3: Update the setting hint and verify frontend tests**

### Task 5: Full verification

**Files:**
- Verify only.

**Interfaces:**
- Consumes: all fixes above.
- Produces: fresh test/build evidence.

- [ ] **Step 1: Run `node --check frontend/app.js frontend/notification-settings.js`**
- [ ] **Step 2: Run `node --test frontend/*.test.js`**
- [ ] **Step 3: Run `cargo test` and `cargo check`**
- [ ] **Step 4: Run `git diff --check` and inspect notification/layout hunks**

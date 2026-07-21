# Agent Notification Policy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop Lotus from replaying Agent commands at startup and make completion notifications and Tab badges follow a quiet, background-only policy.

**Architecture:** Keep legacy `TabSession.command` deserialization compatibility while scrubbing commands at the storage boundary and never restoring them into PTYs. Extract frontend command lifecycle and completion policy into a small UMD module that is directly testable with Node, then wire `app.js` to that module for transient Agent state, duplicate suppression, badge selection, and the 10-second background notification threshold.

**Tech Stack:** Rust 2024, Serde/Serde JSON, GTK/WebKitGTK, browser JavaScript, Node.js built-in `node:test`.

## Global Constraints

- Restore Tab title and working directory, but never automatically restart an Agent.
- Keep existing `sessions.json` readable, including legacy non-null `command` fields.
- Notify only for commands completed in a background Tab with `duration_ms >= 10000`.
- Show a busy badge while running; keep completion/failure badges only on background Tabs and clear them when viewed.
- Agent state applies only to the currently running command.
- Do not add settings UI or new runtime dependencies.

---

## File Structure

- `src/storage.rs`: owns the invariant that persisted/restored session snapshots do not carry executable commands.
- `src/main.rs`: restores Tab metadata without scheduling old commands and persists `command: null`.
- `src/term/manager.rs`: removes the obsolete replay-command field from live Tab metadata.
- `frontend/agent-policy.js`: pure command lifecycle, completion policy, and duplicate-event key functions.
- `frontend/app.js`: DOM/IPC integration using `AgentPolicy`; it does not define policy thresholds.
- `frontend/index.html`: loads `agent-policy.js` before `app.js` and describes the new notification rule.
- `tests/frontend/agent-policy.test.js`: pure JavaScript policy tests.
- `tests/frontend/app-agent-integration.test.js`: script-order and source-wiring smoke tests.
- `README.md`: documents background-only completion notifications.

---

### Task 1: Make session restoration metadata-only

**Files:**
- Modify: `src/storage.rs:446-568`
- Modify: `src/main.rs:675-684,1199-1341`
- Modify: `src/term/manager.rs:35-54,151-169,229-256`
- Test: `src/storage.rs` inline test module

**Interfaces:**
- Consumes: legacy `TabSession { command: Option<String> }` JSON representation.
- Produces: `SessionStore::discard_commands(&mut self)` and a `SessionStore::replace_from(...)` invariant that sets every `TabSession.command` to `None`.

- [ ] **Step 1: Write failing storage compatibility and scrubbing tests**

Append this test module to `src/storage.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{SessionStore, TabSession};
    use std::collections::HashMap;

    #[test]
    fn legacy_session_command_is_read_then_discarded_before_restore() {
        let json = r#"{
            "tabs": [{
                "project_id": 1,
                "title": "pi",
                "cwd": "/tmp/project",
                "command": "pi"
            }],
            "active_by_project": {"1": 0}
        }"#;

        let mut store: SessionStore = serde_json::from_str(json).unwrap();
        assert_eq!(store.tabs[0].command.as_deref(), Some("pi"));

        store.discard_commands();

        assert_eq!(store.tabs[0].command, None);
        assert_eq!(store.tabs[0].title, "pi");
        assert_eq!(store.tabs[0].cwd, "/tmp/project");
    }

    #[test]
    fn replacing_session_snapshot_never_keeps_replayable_commands() {
        let mut store = SessionStore::default();
        let mut active = HashMap::new();
        active.insert(1, 7);

        store.replace_from(
            vec![TabSession {
                project_id: 1,
                title: "pi".into(),
                cwd: "/tmp/project".into(),
                command: Some("pi".into()),
            }],
            &active,
            &[(7, 1)],
        );

        assert_eq!(store.tabs[0].command, None);
        assert_eq!(store.active_by_project.get(&1), Some(&0));
    }
}
```

- [ ] **Step 2: Run the targeted tests and verify the new API is missing**

Run:

```bash
cargo test legacy_session_command_is_read_then_discarded_before_restore
```

Expected: compilation fails because `SessionStore::discard_commands` does not exist.

- [ ] **Step 3: Add the storage-boundary command scrubbing invariant**

Add this method inside `impl SessionStore` in `src/storage.rs`, immediately before `replace_from`:

```rust
    /// Remove executable commands while retaining Tab metadata.
    pub fn discard_commands(&mut self) {
        for tab in &mut self.tabs {
            tab.command = None;
        }
    }
```

Change the start of `replace_from` to:

```rust
    pub fn replace_from(
        &mut self,
        tabs: Vec<TabSession>,
        active_tab_ids: &HashMap<u32, u32>,
        runtime_tabs: &[(u32, u32)],
    ) {
        self.tabs = tabs;
        self.discard_commands();
        self.active_by_project.clear();
```

- [ ] **Step 4: Run both storage tests and verify they pass**

Run:

```bash
cargo test storage::tests
```

Expected: both new tests pass.

- [ ] **Step 5: Remove replay commands from live Tab metadata**

In `src/term/manager.rs`:

1. Remove `command: Option<String>` from `Tab`.
2. Remove `pub command: Option<String>` from `TabInfo`.
3. Remove `command: None` from the `Tab` initializer.
4. Delete `TermManager::set_command`.
5. Remove `command: t.command.clone()` from `list_tabs()`.

The resulting metadata types must be:

```rust
struct Tab {
    handle: PtyHandle,
    title: String,
    project_id: u32,
    cwd: String,
}

#[derive(Debug, Clone)]
pub struct TabInfo {
    pub tab_id: u32,
    pub project_id: u32,
    pub title: String,
    pub cwd: String,
}
```

- [ ] **Step 6: Make persistence and restoration metadata-only**

In `persist_sessions` in `src/main.rs`, construct each session with no command:

```rust
        .map(|t| TabSession {
            project_id: t.project_id,
            title: t.title,
            cwd: t.cwd,
            command: None,
        })
```

Change the `ClientMessage::Ready` restoration block to:

```rust
            if !s.first_tab_created {
                s.first_tab_created = true;
                // Restore Tab metadata only. Commands are never replayed.
                restore_sessions(&mut s);
            }
```

Change `restore_sessions` to return unit, discard legacy commands before consuming the store, and remove all delayed-command collection:

```rust
fn restore_sessions(s: &mut AppState) {
    let mut store = SessionStore::load();
    store.discard_commands();
    let active_by_project = store.active_by_project.clone();
    let mut pending: Vec<TabSession> = store
        .tabs
        .into_iter()
        .filter(|t| s.projects.get(t.project_id).is_some())
        .collect();
```

Within its Tab creation loop, retain only title/cwd restoration:

```rust
        let created = s.manager.as_mut().and_then(|m| {
            match m.create_tab(80, 24, Some(&cwd), pid) {
                Ok(tab_id) => {
                    m.set_title(tab_id, title.clone());
                    Some(tab_id)
                }
                Err(e) => {
                    error!("恢复 tab 失败 (project={}, cwd={}): {}", pid, cwd, e);
                    None
                }
            }
        });
```

Delete the `delayed` vector, the block that pushes `sess.command`, and the final `delayed` expression. Keep `schedule_tab_commands` because the Agents panel still uses it for user-initiated launches.

In `ClientMessage::LaunchAgent`, remove only this stale persistence call:

```rust
m.set_command(tab_id, Some(command.clone()));
```

The subsequent `schedule_tab_commands(...)` call remains unchanged so an explicitly launched Agent still starts immediately.

- [ ] **Step 7: Format and run all Rust tests**

Run:

```bash
cargo fmt
cargo test
```

Expected: all Rust tests pass, and no `set_command` or `TabInfo.command` references remain.

Verify:

```bash
rg -n 'set_command|t\.command|delayed\.push' src
```

Expected: no output.

- [ ] **Step 8: Commit the metadata-only restoration change**

```bash
git add src/storage.rs src/main.rs src/term/manager.rs
git commit -m "fix: stop replaying agent commands on startup"
```

---

### Task 2: Add a testable frontend Agent policy module

**Files:**
- Create: `frontend/agent-policy.js`
- Create: `tests/frontend/agent-policy.test.js`

**Interfaces:**
- Consumes: command text, Agent classification boolean, active-Tab boolean, exit code, and duration in milliseconds.
- Produces: global/CommonJS `AgentPolicy` with `createRuntime()`, `startCommand(runtime, cmd, isAgent)`, `completionKey(cmd, code, durationMs)`, `finishCommand(runtime, key)`, and `completionDisposition({ active, durationMs, ok })`.

- [ ] **Step 1: Write failing lifecycle and notification policy tests**

Create `tests/frontend/agent-policy.test.js`:

```javascript
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
```

- [ ] **Step 2: Run the test and verify the module is missing**

Run:

```bash
node --test tests/frontend/agent-policy.test.js
```

Expected: FAIL with `Cannot find module '../../frontend/agent-policy.js'`.

- [ ] **Step 3: Implement the pure UMD policy module**

Create `frontend/agent-policy.js`:

```javascript
(function attachAgentPolicy(root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  if (root) root.AgentPolicy = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function createAgentPolicy() {
  'use strict';

  const BACKGROUND_NOTIFY_MIN_MS = 10000;

  function createRuntime() {
    return { busy: false, cmd: '', isAgent: false, lastCompletionKey: '' };
  }

  function startCommand(runtime, cmd, isAgent) {
    return {
      busy: true,
      cmd: cmd || '',
      isAgent: Boolean(isAgent),
      lastCompletionKey: '',
    };
  }

  function completionKey(cmd, code, durationMs) {
    return JSON.stringify([String(cmd || ''), code, durationMs]);
  }

  function finishCommand(runtime, key) {
    if (runtime.lastCompletionKey === key) {
      return { runtime, wasAgent: false, duplicate: true };
    }
    return {
      runtime: {
        busy: false,
        cmd: '',
        isAgent: false,
        lastCompletionKey: key,
      },
      wasAgent: Boolean(runtime.isAgent),
      duplicate: false,
    };
  }

  function completionDisposition({ active, durationMs, ok }) {
    const background = !active;
    const validDuration = Number.isFinite(durationMs) && durationMs >= 0;
    return {
      badge: background ? (ok ? 'done' : 'error') : null,
      notify: background && validDuration && durationMs >= BACKGROUND_NOTIFY_MIN_MS,
    };
  }

  return {
    BACKGROUND_NOTIFY_MIN_MS,
    createRuntime,
    startCommand,
    completionKey,
    finishCommand,
    completionDisposition,
  };
});
```

- [ ] **Step 4: Run policy tests and syntax checks**

Run:

```bash
node --test tests/frontend/agent-policy.test.js
node --check frontend/agent-policy.js
```

Expected: six tests pass and the syntax check exits successfully.

- [ ] **Step 5: Commit the policy module**

```bash
git add frontend/agent-policy.js tests/frontend/agent-policy.test.js
git commit -m "test: define quiet agent notification policy"
```

---

### Task 3: Wire transient Agent state into the Lotus frontend

**Files:**
- Modify: `frontend/index.html:81-95,258`
- Modify: `frontend/app.js:40-48,798-808,990-1005,2329-2400`
- Modify: `frontend/styles.css:238,265-267`
- Modify: `README.md:51-55`
- Create: `tests/frontend/app-agent-integration.test.js`

**Interfaces:**
- Consumes: `window.AgentPolicy` loaded by `frontend/index.html` and command events from Rust IPC.
- Produces: transient Agent styling, background-only result badges, one notification per eligible completion, and immediate clearing when the result Tab is viewed.

- [ ] **Step 1: Write failing frontend wiring smoke tests**

Create `tests/frontend/app-agent-integration.test.js`:

```javascript
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
```

- [ ] **Step 2: Run the integration smoke tests and verify they fail**

Run:

```bash
node --test tests/frontend/app-agent-integration.test.js
```

Expected: failures for the missing policy script and missing `AgentPolicy` calls, plus sticky-title patterns still found.

- [ ] **Step 3: Load the policy module before the application**

At the bottom of `frontend/index.html`, replace the single application script with:

```html
  <script src="agent-policy.js"></script>
  <script src="app.js"></script>
```

- [ ] **Step 4: Stop titles from creating permanent Agent state**

In the `title_changed` IPC case in `frontend/app.js`, retain only title updates:

```javascript
    case 'title_changed': {
      const t = terminals.get(msg.tab_id);
      if (t) t.title = msg.title;
      updateTabTitle(msg.tab_id, msg.title);
      break;
    }
```

In `addTabUI`, initialize runtime independently of the title and remove the title-based `has-agent` class:

```javascript
  if (!tabRuntime.has(tabId)) {
    tabRuntime.set(tabId, AgentPolicy.createRuntime());
  }
```

Change `ensureTabRuntime` to:

```javascript
function ensureTabRuntime(tabId) {
  if (!tabRuntime.has(tabId)) {
    tabRuntime.set(tabId, AgentPolicy.createRuntime());
  }
  return tabRuntime.get(tabId);
}
```

- [ ] **Step 5: Replace command start handling with transient state**

Replace `onCommandStarted` with:

```javascript
function onCommandStarted(tabId, cmd) {
  const rt = AgentPolicy.startCommand(
    ensureTabRuntime(tabId),
    cmd,
    isAgentCommand(cmd)
  );
  tabRuntime.set(tabId, rt);

  const tabEl = document.querySelector(`.tab[data-tab-id="${tabId}"]`);
  if (tabEl) tabEl.classList.toggle('has-agent', rt.isAgent);

  if (rt.isAgent) {
    const short = String(cmd).trim().split(/\s+/)[0];
    updateTabTitle(tabId, short);
  }
  setTabBadge(tabId, 'busy', '运行中: ' + (cmd || ''));
}
```

- [ ] **Step 6: Replace completion handling with deduplication and background policy**

Replace `onCommandFinished` with:

```javascript
function onCommandFinished(tabId, cmd, code, durationMs) {
  if (!tabRuntime.has(tabId)) return;

  const previous = tabRuntime.get(tabId);
  const key = AgentPolicy.completionKey(cmd, code, durationMs);
  const finished = AgentPolicy.finishCommand(previous, key);
  if (finished.duplicate) return;
  tabRuntime.set(tabId, finished.runtime);

  const tabEl = document.querySelector(`.tab[data-tab-id="${tabId}"]`);
  if (tabEl) tabEl.classList.remove('has-agent');

  const ok = code === 0;
  const active = getActiveTabId() === tabId;
  const disposition = AgentPolicy.completionDisposition({ active, durationMs, ok });
  const badgeTitle =
    (ok ? '已完成' : '失败') + ': ' + (cmd || '') +
    (durationMs ? ` (${Math.round(durationMs / 100) / 10}s)` : '');
  setTabBadge(tabId, disposition.badge, disposition.badge ? badgeTitle : '');

  if (disposition.notify) {
    const isAgent = finished.wasAgent || isAgentCommand(cmd);
    const status = ok ? '完成' : '失败';
    const short = String(cmd || '').replace(/\n/g, ' ').slice(0, 60);
    sendToBackend({
      type: 'desktop_notify',
      title: isAgent ? `Agent ${status}` : `命令${status}`,
      body: short + ` · ${(durationMs / 1000).toFixed(1)}s`,
    });
  }
}
```

Keep the existing `switchTab` call to `clearTabDoneBadge(tabId)`; it already implements “viewing clears background result while preserving busy.” Remove the old 2.5-second foreground timeout and the old unconditional Agent notification branch as part of the function replacement.

- [ ] **Step 7: Update comments and user-facing behavior descriptions**

In `frontend/styles.css`, change the relevant comments to:

```css
/* tab 状态徽章（运行中 / 后台完成） */
```

and:

```css
/* 仅当前命令是 Agent 时强调标题 */
.tab.has-agent .tab-title {
  color: var(--accent);
}
```

In `frontend/index.html`, change the Agents intro text to:

```html
        一键在新标签启动编码 Agent CLI。Lotus 会跟踪命令状态，并在后台任务运行至少 10 秒后完成时通知你。
```

In `README.md`, replace the completion notification bullet with:

```markdown
- **完成通知**：仅后台运行至少 10 秒的任务结束时发送桌面通知（`notify-send`）
```

- [ ] **Step 8: Run frontend tests and syntax checks**

Run:

```bash
node --test tests/frontend/*.test.js
node --check frontend/agent-policy.js
node --check frontend/app.js
```

Expected: ten frontend tests pass and both syntax checks exit successfully.

- [ ] **Step 9: Commit the frontend integration**

```bash
git add frontend/index.html frontend/app.js frontend/styles.css README.md tests/frontend/app-agent-integration.test.js
git commit -m "fix: quiet background agent notifications"
```

---

### Task 4: Run full regression verification

**Files:**
- Verify only; no planned source changes.

**Interfaces:**
- Consumes: completed backend and frontend changes from Tasks 1-3.
- Produces: evidence that formatting, tests, compilation, policy wiring, and session behavior are consistent.

- [ ] **Step 1: Run formatting and whitespace checks**

```bash
cargo fmt --check
git diff --check
```

Expected: both commands exit successfully with no output.

- [ ] **Step 2: Run all automated tests**

```bash
cargo test
node --test tests/frontend/*.test.js
```

Expected: every Rust and frontend test passes.

- [ ] **Step 3: Run compile and JavaScript syntax checks**

```bash
cargo check
node --check frontend/agent-policy.js
node --check frontend/app.js
```

Expected: all commands exit successfully.

- [ ] **Step 4: Verify forbidden replay and sticky-state patterns are absent**

```bash
rg -n 'set_command|t\.command|delayed\.push|isAgent:\s*isAgentCommand\(title\)|isAgentCommand\(msg\.title\)|isAgent\s*\|\|\s*\(notFocused' src frontend
```

Expected: no output.

- [ ] **Step 5: Inspect the final change set**

```bash
git status --short
git log --oneline -4
git diff HEAD~3 --stat
git diff HEAD~3 -- src/storage.rs src/main.rs src/term/manager.rs frontend/agent-policy.js frontend/app.js frontend/index.html README.md
```

Expected: working tree is clean; the three implementation commits are present; the diff is limited to session command replay, Agent state, notification policy, tests, and matching documentation.

# Terminal Output Backpressure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep Agent CLI and other high-volume terminal output responsive and visually correct without dropping UTF-8 bytes or ANSI control sequences.

**Architecture:** Preserve PTY output as bytes through the browser boundary and let xterm.js perform its stream-aware UTF-8 decoding. Add a per-tab, sequence-numbered acknowledgement loop so Rust sends at most one bounded chunk per terminal until xterm has parsed it; use bounded Rust channels so this acknowledgement propagates back to the PTY reader as real backpressure.

**Tech Stack:** Rust 2024, `std::sync::mpsc`, Serde JSON IPC, browser JavaScript, xterm.js 5.5, Node.js built-in test runner.

## Global Constraints

- Preserve every PTY byte and its per-tab order; never truncate terminal output.
- Limit each browser-bound output chunk to 64 KiB.
- Keep no more than 512 KiB of unsent output in the GTK-side flow controller before pausing event-channel draining.
- Acknowledge a chunk only after the xterm.js `write` callback reports that parsing completed.
- Retain in-flight bytes until acknowledgement and requeue them if WebKit rejects the JavaScript delivery.
- Delay process-exit tab removal until its final queued output has been parsed.
- Suppress high-frequency `output` / `output_ack` console logging.
- Do not change terminal layout, theme, Agent CLI behavior, or existing IME handling.

---

### Task 1: Browser binary output writer

**Files:**
- Create: `frontend/terminal-output.js`
- Create: `frontend/terminal-output.test.js`
- Modify: `frontend/index.html`
- Modify: `frontend/app.js`

**Interfaces:**
- Consumes: base64-encoded PTY bytes from `ServerMessage::Output`.
- Produces: `writeTerminalOutput(term, base64, onParsed)` and `{ type: "output_ack", tab_id, seq }` after xterm parsing.

- [x] **Step 1: Write the failing browser helper tests**

```javascript
test('writes decoded PTY bytes without converting chunks to strings', () => {
  let parsed;
  const term = { write(data, callback) { parsed = callback; assert.ok(data instanceof Uint8Array); } };
  writeTerminalOutput(term, '5Lit5paH', () => { acknowledged = true; });
  assert.equal(acknowledged, false);
  parsed();
  assert.equal(acknowledged, true);
});
```

- [x] **Step 2: Run the browser helper tests to verify RED**

Run: `node --test frontend/terminal-output.test.js`
Expected: FAIL because `frontend/terminal-output.js` does not yet export `writeTerminalOutput`.

- [x] **Step 3: Implement byte-preserving xterm writes**

```javascript
function writeTerminalOutput(term, base64, onParsed) {
  const binary = atob(base64);
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
  term.write(bytes, onParsed);
}
```

Load the helper before `app.js`; replace `decodeURIComponent(escape(...))` with the helper and send `output_ack` from the write callback.

- [x] **Step 4: Run the browser helper tests to verify GREEN**

Run: `node --test frontend/terminal-output.test.js`
Expected: PASS with all browser output tests green.

### Task 2: Sequence-aware Rust output flow controller

**Files:**
- Create: `src/term/output_flow.rs`
- Modify: `src/term/mod.rs`
- Modify: `src/ipc.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: ordered `(tab_id, Vec<u8>)` PTY events and `OutputAck { tab_id, seq }` client messages.
- Produces: `OutputChunk { tab_id, seq, data }`, at most one in flight per tab and at most 64 KiB per chunk.

- [x] **Step 1: Write failing flow-controller and IPC tests**

```rust
#[test]
fn waits_for_matching_ack_before_releasing_more_output() {
    let mut flow = OutputFlow::new(4, 8);
    flow.push(7, b"abcdefghij".to_vec());
    let first = flow.take_ready();
    assert_eq!(first[0].data, b"abcd");
    assert!(flow.take_ready().is_empty());
    assert!(!flow.acknowledge(7, first[0].seq + 1));
    assert!(flow.take_ready().is_empty());
    assert!(flow.acknowledge(7, first[0].seq));
    assert_eq!(flow.take_ready()[0].data, b"efgh");
}
```

Also assert that `{"type":"output_ack","tab_id":7,"seq":3}` deserializes successfully.

- [x] **Step 2: Run focused Rust tests to verify RED**

Run: `cargo test output_flow -- --nocapture`

Run: `cargo test ipc::tests -- --nocapture`
Expected: FAIL because output acknowledgements and `OutputFlow` do not exist.

- [x] **Step 3: Implement the minimal flow controller and protocol**

Use a `HashMap<u32, TabOutput>` with a `VecDeque<u8>` and `Option<u64>` in-flight sequence per tab. Drain at most 64 KiB for a ready tab, ignore stale acknowledgements, remove state when a tab closes, and stop pulling PTY events while unsent bytes are at the 512 KiB high-water mark.

- [x] **Step 4: Run focused Rust tests to verify GREEN**

Run: `cargo test output_flow -- --nocapture`

Run: `cargo test ipc::tests -- --nocapture`
Expected: PASS with chunk ordering, one-in-flight, stale-ACK, saturation, and protocol tests green.

### Task 3: Propagate backpressure to the PTY and verify the regression

**Files:**
- Modify: `src/pty.rs`
- Modify: `src/term/manager.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: flow-controller saturation and bounded channel capacity.
- Produces: bounded PTY-to-bridge and bridge-to-GTK event queues that naturally block the PTY reader during sustained output.

- [x] **Step 1: Replace unbounded output channels with bounded synchronous channels**

Use `sync_channel::<PtyOutput>(32)` per tab and `sync_channel::<TermEvent>(256)` for the shared manager event stream. Update sender types to `SyncSender`.

- [x] **Step 2: Batch and gate output in the GTK tick**

Enqueue PTY bytes into `OutputFlow`, emit ready 64 KiB chunks as sequence-bearing `ServerMessage::Output`, and process `OutputAck` by releasing only its matching per-tab in-flight chunk.

- [x] **Step 3: Run formatting and complete verification**

Run: `rustfmt --edition 2024 --check src/term/output_flow.rs`
Expected: exit 0. (The repository has pre-existing `cargo fmt --check` differences in untouched files, so verification is scoped to the new Rust module.)

Run: `node --test frontend/terminal-output.test.js`
Expected: all tests pass.

Run: `cargo test -- --nocapture`
Expected: all tests pass.

Run: `cargo build`
Expected: exit 0 with no compilation errors.

- [x] **Step 4: Review the final diff for scope and byte-order safety**

Run: `git diff --check && git diff --stat && git status --short`
Expected: no whitespace errors; only the planned terminal-output, IPC, PTY, tests, HTML include, and plan files are changed.

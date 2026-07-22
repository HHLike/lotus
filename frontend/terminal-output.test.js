const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

let shouldLogIpcMessage;
let writeTerminalOutput;
try {
  ({ shouldLogIpcMessage, writeTerminalOutput } = require('./terminal-output.js'));
} catch (_) {
  // The first RED run intentionally happens before the helper exists.
}

test('writes decoded PTY bytes without converting chunks to strings', () => {
  assert.equal(typeof writeTerminalOutput, 'function');

  let parsedCallback = null;
  let acknowledged = false;
  const term = {
    write(data, callback) {
      assert.ok(data instanceof Uint8Array);
      assert.deepEqual([...data], [0xe4, 0xb8, 0xad, 0xe6, 0x96, 0x87]);
      parsedCallback = callback;
    },
  };

  writeTerminalOutput(term, '5Lit5paH', () => {
    acknowledged = true;
  });

  assert.equal(acknowledged, false);
  assert.equal(typeof parsedCallback, 'function');
  parsedCallback();
  assert.equal(acknowledged, true);
});

test('preserves an incomplete UTF-8 sequence as raw bytes', () => {
  assert.equal(typeof writeTerminalOutput, 'function');

  let written;
  const term = {
    write(data) {
      written = data;
    },
  };

  writeTerminalOutput(term, '5Lg=', () => {});

  assert.deepEqual([...written], [0xe4, 0xb8]);
});

test('suppresses high-frequency terminal stream logs', () => {
  assert.equal(typeof shouldLogIpcMessage, 'function');
  assert.equal(shouldLogIpcMessage({ type: 'output' }), false);
  assert.equal(shouldLogIpcMessage({ type: 'output_ack' }), false);
  assert.equal(shouldLogIpcMessage({ type: 'tab_created' }), true);
});

test('terminal gutter is owned by xterm so FitAddon subtracts it from visible rows', () => {
  const css = fs.readFileSync(path.join(__dirname, 'styles.css'), 'utf8');
  const paneRule = css.match(/\.terminal-pane\s*\{([^}]*)\}/)?.[1] || '';
  const xtermRule = css.match(/\.xterm\s*\{([^}]*)\}/)?.[1] || '';

  assert.doesNotMatch(paneRule, /padding:\s*[1-9]/);
  assert.match(xtermRule, /padding:\s*4px/);
});

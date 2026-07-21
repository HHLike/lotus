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

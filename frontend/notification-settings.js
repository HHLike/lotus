(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) {
    module.exports = api;
  } else {
    root.NotificationSettings = api;
  }
}(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  function shouldSend(config, kind, eligible) {
    if (kind === 'agent') {
      return Boolean(eligible) && config?.agent_notifications_enabled !== false;
    }
    if (kind === 'command') {
      return config?.command_notifications_enabled === true;
    }
    return false;
  }

  return { shouldSend };
}));

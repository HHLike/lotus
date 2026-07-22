(function (root, factory) {
  const api = factory(root);
  if (typeof module === 'object' && module.exports) {
    module.exports = api;
  } else {
    root.shouldLogIpcMessage = api.shouldLogIpcMessage;
    root.writeTerminalOutput = api.writeTerminalOutput;
  }
}(typeof globalThis !== 'undefined' ? globalThis : this, function (root) {
  function shouldLogIpcMessage(message) {
    return message && message.type !== 'output' && message.type !== 'output_ack';
  }

  function writeTerminalOutput(term, base64, onParsed) {
    const binary = root.atob(base64);
    const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
    term.write(bytes, onParsed);
  }

  return { shouldLogIpcMessage, writeTerminalOutput };
}));

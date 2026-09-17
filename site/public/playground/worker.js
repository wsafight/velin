import init, {check, run, PlaygroundSession} from './pkg/velin_wasm.js';

const ready = init();
let debugSession = null;

ready
  .then(() => globalThis.postMessage({type: 'ready'}))
  .catch(error => globalThis.postMessage({type: 'init-error', error: String(error)}));

globalThis.addEventListener('message', async event => {
  const message = event.data;
  if (message?.type !== 'request' || !Number.isInteger(message.id)) return;

  try {
    await ready;
    let serialized;
    if (message.operation === 'check') {
      serialized = check(String(message.source ?? ''));
    } else if (message.operation === 'run') {
      serialized = run(String(message.source ?? ''), String(message.replies ?? '[]'));
    } else if (message.operation === 'debug-start') {
      debugSession?.free();
      debugSession = new PlaygroundSession(String(message.source ?? ''));
      serialized = debugSession.state();
    } else if (message.operation === 'debug-resume') {
      if (!debugSession) throw new Error('Start a debug session first.');
      serialized = debugSession.resume(String(message.replies ?? '[]'));
    } else if (message.operation === 'debug-step') {
      if (!debugSession) throw new Error('Start a debug session first.');
      serialized = debugSession.step(String(message.replies ?? '[]'));
    } else if (message.operation === 'debug-snapshot') {
      if (!debugSession) throw new Error('Start a debug session first.');
      serialized = debugSession.snapshot();
    } else if (message.operation === 'debug-restore') {
      if (!debugSession) throw new Error('Start a debug session first.');
      serialized = debugSession.restore(Number(message.snapshot));
    } else {
      throw new Error(`Unknown worker operation: ${String(message.operation)}`);
    }
    globalThis.postMessage({type: 'result', id: message.id, result: JSON.parse(serialized)});
  } catch (error) {
    globalThis.postMessage({type: 'error', id: message.id, error: String(error)});
  }
});

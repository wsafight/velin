import init, {check, run} from './pkg/velin_wasm.js';

const ready = init();

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
    } else {
      throw new Error(`Unknown worker operation: ${String(message.operation)}`);
    }
    globalThis.postMessage({type: 'result', id: message.id, result: JSON.parse(serialized)});
  } catch (error) {
    globalThis.postMessage({type: 'error', id: message.id, error: String(error)});
  }
});

const SAMPLE = `default hp = 30

choice = perform ask("Drink the potion?")
if choice == 1:
    set hp = hp + 10
    perform say("You feel restored.")
else:
    perform say("You leave it untouched.")

perform say("HP is now")
perform say(hp)
`;

const ENGINE_TIMEOUT_MS = 5_000;

const elements = {
  source: document.getElementById('source'),
  replies: document.getElementById('replies'),
  run: document.getElementById('run'),
  check: document.getElementById('check'),
  stop: document.getElementById('stop'),
  language: document.getElementById('language'),
  theme: document.getElementById('theme'),
  menu: document.getElementById('mobile-menu'),
  menuToggle: document.getElementById('menu-toggle'),
  diagnostics: document.getElementById('diagnostics'),
  output: document.getElementById('output'),
};

class EngineWorker {
  constructor(url) {
    this.url = url;
    this.worker = null;
    this.ready = null;
    this.rejectReady = null;
    this.pending = new Map();
    this.nextId = 1;
  }

  start() {
    if (this.ready) return this.ready;

    const worker = new Worker(this.url, {type: 'module'});
    this.worker = worker;
    this.ready = new Promise((resolve, reject) => {
      this.rejectReady = reject;
      worker.addEventListener('message', event => {
        if (worker !== this.worker) return;
        const message = event.data;
        if (message?.type === 'ready') {
          this.rejectReady = null;
          resolve();
          return;
        }
        if (message?.type === 'init-error') {
          this.reset(new Error(message.error));
          return;
        }
        if (message?.type !== 'result' && message?.type !== 'error') return;
        const request = this.pending.get(message.id);
        if (!request) return;
        clearTimeout(request.timer);
        this.pending.delete(message.id);
        if (message.type === 'result') request.resolve(message.result);
        else request.reject(new Error(message.error));
      });
      worker.addEventListener('error', event => {
        if (worker === this.worker) this.reset(new Error(event.message || 'Wasm worker failed'));
      });
    });
    return this.ready;
  }

  async request(operation, payload) {
    await this.start();
    return new Promise((resolve, reject) => {
      const id = this.nextId++;
      const timer = setTimeout(() => {
        this.reset(new Error(`${operation === 'run' ? 'Execution' : 'Check'} timed out after 5 seconds.`));
      }, ENGINE_TIMEOUT_MS);
      this.pending.set(id, {resolve, reject, timer});
      this.worker.postMessage({type: 'request', id, operation, ...payload});
    });
  }

  stop(message) {
    this.reset(new Error(message));
  }

  reset(error) {
    this.worker?.terminate();
    this.worker = null;
    this.ready = null;
    this.rejectReady?.(error);
    this.rejectReady = null;
    for (const request of this.pending.values()) {
      clearTimeout(request.timer);
      request.reject(error);
    }
    this.pending.clear();
  }
}

const engine = new EngineWorker(new URL('./worker.js', import.meta.url));

function currentLang() {
  return document.documentElement.dataset.lang === 'zh' ? 'zh' : 'en';
}

function applyLang(lang, persist = true) {
  document.documentElement.dataset.lang = lang;
  document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en';
  elements.language.setAttribute('aria-label', lang === 'zh' ? 'Switch to English' : 'Switch to Chinese');
  if (persist) {
    localStorage.setItem('velin-lang', lang);
    const url = new URL(location.href);
    if (lang === 'zh') url.searchParams.set('lang', 'zh');
    else url.searchParams.delete('lang');
    history.replaceState(null, '', `${url.pathname}${url.search}${url.hash}`);
  }
}

function renderDiagnostics(diagnostics) {
  elements.diagnostics.replaceChildren();
  elements.diagnostics.classList.toggle('is-empty', diagnostics.length === 0);
  if (!diagnostics.length) {
    const item = document.createElement('li');
    item.className = 'empty';
    item.textContent = currentLang() === 'zh' ? '没有问题。' : 'No problems.';
    elements.diagnostics.append(item);
    return;
  }
  for (const diagnostic of diagnostics) {
    const item = document.createElement('li');
    item.className = diagnostic.severity;
    const where = document.createElement('span');
    where.className = 'where';
    where.textContent = `${diagnostic.line}:${diagnostic.column}`;
    item.append(where, document.createTextNode(diagnostic.message));
    elements.diagnostics.append(item);
  }
}

function renderOutput(lines) {
  elements.output.textContent = (lines ?? []).join('\n');
  elements.output.dataset.empty = elements.output.textContent ? 'false' : 'true';
}

function busy(active, stoppable = false) {
  elements.run.disabled = active;
  elements.check.disabled = active;
  elements.stop.disabled = !stoppable;
  document.body.classList.toggle('is-busy', active);
}

async function onCheck() {
  busy(true, true);
  try {
    const result = await engine.request('check', {source: elements.source.value});
    renderDiagnostics(result.diagnostics);
    renderOutput([]);
  } catch (error) {
    renderOutput([`Error: ${String(error.message || error)}`]);
  } finally {
    busy(false);
  }
}

async function onRun() {
  busy(true, true);
  try {
    const result = await engine.request('run', {
      source: elements.source.value,
      replies: elements.replies.value,
    });
    renderDiagnostics(result.diagnostics);
    const lines = [...(result.output ?? [])];
    if (result.error) lines.push(`Error: ${result.error}`);
    renderOutput(lines);
  } catch (error) {
    renderOutput([`Error: ${String(error.message || error)}`]);
  } finally {
    busy(false);
  }
}

function onStop() {
  const message = currentLang() === 'zh' ? '执行已停止。' : 'Execution stopped.';
  engine.stop(message);
}

function showLoadError(error) {
  const banner = document.createElement('p');
  banner.className = 'banner';
  banner.textContent = `Could not load the Wasm engine: ${String(error)}`;
  document.querySelector('main').prepend(banner);
  busy(true);
}

function setMenu(open) {
  elements.menu?.classList.toggle('is-open', open);
  elements.menu?.setAttribute('aria-hidden', open ? 'false' : 'true');
  elements.menuToggle?.setAttribute('aria-expanded', open ? 'true' : 'false');
  elements.menuToggle?.setAttribute('aria-label', open ? 'Close menu' : 'Open menu');
  document.body.classList.toggle('menu-open', open);
}

async function main() {
  elements.source.value = SAMPLE;
  elements.language.addEventListener('click', () => {
    applyLang(currentLang() === 'zh' ? 'en' : 'zh');
    if (elements.diagnostics.classList.contains('is-empty')) renderDiagnostics([]);
  });
  elements.theme?.addEventListener('click', () => {
    const theme = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark';
    document.documentElement.dataset.theme = theme;
    localStorage.setItem('velin-theme', theme);
  });
  elements.menuToggle?.addEventListener('click', () => setMenu(!elements.menu?.classList.contains('is-open')));
  elements.menu?.querySelectorAll('a').forEach(link => link.addEventListener('click', () => setMenu(false)));
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape') setMenu(false);
  });
  applyLang(currentLang(), false);
  busy(true);
  try {
    await engine.start();
  } catch (error) {
    showLoadError(error);
    return;
  }
  busy(false);
  elements.run.addEventListener('click', onRun);
  elements.check.addEventListener('click', onCheck);
  elements.stop.addEventListener('click', onStop);
  onCheck();
}

main();

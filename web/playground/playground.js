import init, {check, run} from './pkg/velin_wasm.js';

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

const elements = {
  source: document.getElementById('source'),
  replies: document.getElementById('replies'),
  run: document.getElementById('run'),
  check: document.getElementById('check'),
  language: document.getElementById('language'),
  theme: document.getElementById('theme'),
  menu: document.getElementById('mobile-menu'),
  menuToggle: document.getElementById('menu-toggle'),
  diagnostics: document.getElementById('diagnostics'),
  output: document.getElementById('output'),
};

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

function busy(active) {
  elements.run.disabled = active;
  elements.check.disabled = active;
  document.body.classList.toggle('is-busy', active);
}

function onCheck() {
  busy(true);
  try {
    const result = JSON.parse(check(elements.source.value));
    renderDiagnostics(result.diagnostics);
    renderOutput([]);
  } finally {
    busy(false);
  }
}

function onRun() {
  busy(true);
  try {
    const result = JSON.parse(run(elements.source.value, elements.replies.value));
    renderDiagnostics(result.diagnostics);
    const lines = [...(result.output ?? [])];
    if (result.error) lines.push(`Error: ${result.error}`);
    renderOutput(lines);
  } finally {
    busy(false);
  }
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
    await init();
  } catch (error) {
    showLoadError(error);
    return;
  }
  busy(false);
  elements.run.addEventListener('click', onRun);
  elements.check.addEventListener('click', onCheck);
  onCheck();
}

main();

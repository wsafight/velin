import {langFromStorage, ui, type Lang} from '../i18n';

const root = document.documentElement;
const header = document.querySelector<HTMLElement>('#site-header');
const menu = document.querySelector<HTMLElement>('#mobile-menu');
const menuToggle = document.querySelector<HTMLButtonElement>('#menu-toggle');
let lastScroll = window.scrollY;

function currentLang(): Lang {
  return langFromStorage(root.dataset.lang || localStorage.getItem('velin-lang'));
}

function copy(lang = currentLang()) {
  return ui[lang];
}

function applyLang(lang: Lang, persist = true) {
  root.dataset.lang = lang;
  root.lang = lang === 'zh' ? 'zh-CN' : 'en';
  if (persist) {
    localStorage.setItem('velin-lang', lang);
    const url = new URL(location.href);
    if (lang === 'zh') url.searchParams.set('lang', 'zh');
    else url.searchParams.delete('lang');
    history.replaceState(null, '', `${url.pathname}${url.search}${url.hash}`);
  }
  const pageTitle = root.getAttribute(lang === 'zh' ? 'data-title-zh' : 'data-title-en');
  const pageDescription = root.getAttribute(lang === 'zh' ? 'data-desc-zh' : 'data-desc-en');
  if (pageTitle) document.title = pageTitle;
  const meta = document.querySelector('meta[name="description"]');
  if (meta && pageDescription) meta.setAttribute('content', pageDescription);
  document.querySelectorAll<HTMLElement>('[data-i18n-aria]').forEach(node => {
    const key = node.dataset.i18nAria as keyof typeof ui.en | undefined;
    if (key) node.setAttribute('aria-label', ui[lang][key]);
  });
  if (!menu?.classList.contains('is-open')) menuToggle?.setAttribute('aria-label', copy(lang).menuOpen);
  document.querySelectorAll<HTMLButtonElement>('.copy-code').forEach(button => {
    if (button.textContent !== copy(lang).copied) {
      button.textContent = copy(lang).copy;
      button.setAttribute('aria-label', copy(lang).copy);
    }
  });
  watchToc();
}

function setMenu(open: boolean) {
  menu?.classList.toggle('is-open', open);
  menu?.setAttribute('aria-hidden', open ? 'false' : 'true');
  menuToggle?.setAttribute('aria-expanded', open ? 'true' : 'false');
  menuToggle?.setAttribute('aria-label', open ? copy().menuClose : copy().menuOpen);
  document.body.classList.toggle('menu-open', open);
  header?.classList.toggle('menu-open', open);
}

function moveIndicator(target?: HTMLElement | null) {
  const nav = document.querySelector<HTMLElement>('.desktop-nav');
  const indicator = document.querySelector<HTMLElement>('.nav-indicator');
  if (!nav || !indicator || !target) {
    indicator?.style.setProperty('opacity', '0');
    return;
  }
  const navBox = nav.getBoundingClientRect();
  const box = target.getBoundingClientRect();
  indicator.style.width = `${box.width}px`;
  indicator.style.transform = `translateX(${box.left - navBox.left}px)`;
  indicator.style.opacity = '1';
}

header?.querySelectorAll('.desktop-nav a').forEach(link => {
  link.addEventListener('mouseenter', () => moveIndicator(link as HTMLElement));
});
header?.querySelector('.desktop-nav')?.addEventListener('mouseleave', () => {
  moveIndicator(header.querySelector('.desktop-nav a[aria-current="page"]') as HTMLElement | null);
});
moveIndicator(header?.querySelector('.desktop-nav a[aria-current="page"]') as HTMLElement | null);

window.addEventListener('scroll', () => {
  if (!header) return;
  const current = window.scrollY;
  header.classList.toggle('is-scrolled', current > 16);
  header.classList.toggle('is-hidden', current > lastScroll && current > 80 && !document.body.classList.contains('menu-open'));
  lastScroll = current;
}, {passive: true});

menuToggle?.addEventListener('click', () => setMenu(!menu?.classList.contains('is-open')));
menu?.querySelectorAll('a').forEach(link => link.addEventListener('click', () => setMenu(false)));

document.querySelectorAll<HTMLButtonElement>('[data-theme-toggle]').forEach(button => {
  button.addEventListener('click', () => {
    const theme = root.dataset.theme === 'dark' ? 'light' : 'dark';
    root.dataset.theme = theme;
    localStorage.setItem('velin-theme', theme);
  });
});

document.querySelectorAll<HTMLButtonElement>('[data-lang-toggle]').forEach(button => {
  button.addEventListener('click', () => applyLang(currentLang() === 'zh' ? 'en' : 'zh'));
});

document.addEventListener('keydown', event => {
  if (event.key === 'Escape') setMenu(false);
});

document.querySelectorAll<HTMLElement>('.capability-panel').forEach(panel => {
  const activate = () => {
    document.querySelectorAll('.capability-panel.is-active').forEach(item => item.classList.remove('is-active'));
    panel.classList.add('is-active');
  };
  panel.addEventListener('mouseenter', activate);
  panel.addEventListener('focusin', activate);
  panel.querySelector('button')?.addEventListener('click', activate);
});

document.querySelectorAll<HTMLElement>('.prose pre').forEach(block => {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'copy-code';
  button.textContent = copy().copy;
  button.setAttribute('aria-label', copy().copy);
  button.addEventListener('click', async () => {
    const code = block.querySelector('code')?.textContent || '';
    await navigator.clipboard.writeText(code);
    button.textContent = copy().copied;
    button.setAttribute('aria-label', copy().copied);
    window.setTimeout(() => {
      button.textContent = copy().copy;
      button.setAttribute('aria-label', copy().copy);
    }, 1600);
  });
  block.append(button);
});

document.querySelectorAll('.prose.i18n-zh :is(h2,h3,h4)[id]').forEach(heading => {
  if (!heading.id.startsWith('zh-')) heading.id = `zh-${heading.id}`;
});
document.querySelectorAll('.docs-toc.i18n-zh a[href^="#"]').forEach(link => {
  const hash = decodeURIComponent(link.getAttribute('href')?.slice(1) || '');
  if (hash && !hash.startsWith('zh-')) link.setAttribute('href', `#zh-${hash}`);
});

let tocObserver: IntersectionObserver | undefined;
function watchToc() {
  tocObserver?.disconnect();
  const links = [...document.querySelectorAll<HTMLAnchorElement>(`.docs-toc.i18n-${currentLang()} a`)];
  const headings = links.map(link => document.querySelector(decodeURIComponent(link.hash))).filter((node): node is HTMLElement => node instanceof HTMLElement);
  if (!links.length || !headings.length) return;
  tocObserver = new IntersectionObserver(entries => {
    const visible = entries.filter(entry => entry.isIntersecting).at(-1);
    if (!visible?.target.id) return;
    links.forEach(link => link.classList.toggle('is-active', decodeURIComponent(link.hash) === `#${visible.target.id}`));
  }, {rootMargin: '-20% 0px -70%', threshold: 0.1});
  headings.forEach(heading => tocObserver?.observe(heading));
}

applyLang(currentLang(), false);
watchToc();
